use std::collections::HashSet;

use anyhow::{Context as _, anyhow};
use serde::Deserialize;
use serde_json::Value;
use serenity::all::{
    GuildId, Http, Member, PartialGuild, Permissions, Role, RoleId, Timestamp, UserId,
};
use serenity::http::{LightMethod, Request, Route, StatusCode};

use crate::discord::executor::{member_rank, rank_is_higher, role_rank};
use crate::discord::reaction_roles::is_privileged_role_permissions;
use crate::storage::validate_snowflake;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DutyAuthorizationError {
    #[error("{0}")]
    Denied(&'static str),
    #[error("Could not verify current Discord duty authorization: {0}")]
    Unavailable(#[from] anyhow::Error),
}

use DutyAuthorizationError::Denied;

type AuthorizationResult<T> = Result<T, DutyAuthorizationError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemovalAuthorization {
    Present,
    Deleted,
}

pub(crate) struct DutyRoleAuthorization {
    guild: PartialGuild,
    bot: Member,
    unknown_permissions: HashSet<RoleId>,
}

impl DutyRoleAuthorization {
    pub(crate) async fn fetch(http: &Http, guild_id: GuildId) -> AuthorizationResult<Self> {
        let response = http
            .fire(Request::new(Route::Guild { guild_id }, LightMethod::Get))
            .await
            .context("fetch Discord guild and its roles")?;
        let (guild, unknown_permissions) = decode_guild(guild_id, response)?;
        let bot = http
            .get_current_user()
            .await
            .context("fetch current Discord bot identity")?;
        if !bot.bot {
            return Err(anyhow!("Discord did not return a bot identity").into());
        }
        let bot = fetch_member(http, &guild, bot.id).await?;
        if !bot.user.bot {
            return Err(anyhow!("Discord did not return the bot's guild membership").into());
        }

        Ok(Self {
            guild,
            bot,
            unknown_permissions,
        })
    }

    pub(crate) async fn fetch_member(
        &self,
        http: &Http,
        user: UserId,
    ) -> AuthorizationResult<Member> {
        fetch_member(http, &self.guild, user).await
    }

    pub(crate) async fn authorize_settings(
        &self,
        http: &Http,
        actor: UserId,
        previous_on_call: Option<&str>,
        on_call: Option<&str>,
        staff: Option<&str>,
    ) -> AuthorizationResult<()> {
        let previous_role = previous_on_call
            .map(parse_role_id)
            .transpose()?
            .and_then(|role| self.guild.roles.get(&role));
        let actor = self.fetch_member(http, actor).await?;

        // A deleted previous role has no hierarchy to check. Changing the policy still
        // requires current role-management permission.
        self.authorize_actor(&actor, previous_role)?;
        if let Some(role) = on_call {
            self.authorize_staff_role(staff)?;
            self.authorize_assignment(parse_role_id(role)?, &actor)?;
        }
        Ok(())
    }

    pub(crate) async fn authorize_grant(
        &self,
        http: &Http,
        role: RoleId,
        staff: Option<&str>,
        actor: Option<&str>,
    ) -> AuthorizationResult<()> {
        let actor = actor.ok_or(Denied(
            "Duty role policy has no recorded approval. An authorized administrator must save the settings again.",
        ))?;
        let actor = validate_snowflake("duty approval actor", actor)
            .map(UserId::new)
            .map_err(|_| Denied("Duty role policy has an invalid approval actor"))?;

        self.authorize_staff_role(staff)?;
        let actor = self.fetch_member(http, actor).await?;
        self.authorize_assignment(role, &actor)
    }

    pub(crate) fn authorize_staff_role(&self, role: Option<&str>) -> AuthorizationResult<()> {
        if let Some(role) = role
            && !self.guild.roles.contains_key(&parse_role_id(role)?)
        {
            return Err(Denied("Duty staff role does not exist in this guild"));
        }
        Ok(())
    }

    pub(crate) fn authorize_removal(
        &self,
        role: RoleId,
        tracked: bool,
    ) -> AuthorizationResult<RemovalAuthorization> {
        let Some(role) = self.guild.roles.get(&role) else {
            return Ok(RemovalAuthorization::Deleted);
        };

        self.authorize_bot(role)?;
        if !tracked {
            self.authorize_safe_permissions(role)?;
        }

        Ok(RemovalAuthorization::Present)
    }

    fn authorize_assignment(&self, role: RoleId, actor: &Member) -> AuthorizationResult<()> {
        let role = self
            .guild
            .roles
            .get(&role)
            .ok_or(Denied("On-call role does not exist in this guild"))?;

        self.authorize_bot(role)?;
        self.authorize_safe_permissions(role)?;
        self.authorize_actor(actor, Some(role))
    }

    fn authorize_actor(&self, actor: &Member, role: Option<&Role>) -> AuthorizationResult<()> {
        if !can_manage_roles(&self.guild, actor) {
            return Err(Denied(
                "Duty role policy requires current Manage Roles or Administrator permission",
            ));
        }
        if let Some(role) = role
            && actor.user.id != self.guild.owner_id
            && !rank_is_higher(member_rank(actor, &self.guild.roles), role_rank(role))
        {
            return Err(Denied(
                "The approving member's highest role must be above the on-call role",
            ));
        }
        Ok(())
    }

    fn authorize_bot(&self, role: &Role) -> AuthorizationResult<()> {
        if role.id == RoleId::new(self.guild.id.get()) || role.managed {
            return Err(Denied(
                "@everyone and managed roles cannot be changed by Duty",
            ));
        }
        if !can_manage_roles(&self.guild, &self.bot) {
            return Err(Denied(
                "The bot requires current Manage Roles or Administrator permission to change a duty role",
            ));
        }
        if !rank_is_higher(member_rank(&self.bot, &self.guild.roles), role_rank(role)) {
            return Err(Denied(
                "The bot's highest role must be above the on-call role",
            ));
        }
        Ok(())
    }

    fn authorize_safe_permissions(&self, role: &Role) -> AuthorizationResult<()> {
        if self.unknown_permissions.contains(&role.id) {
            return Err(Denied("On-call role has unrecognized Discord permissions"));
        }
        if is_privileged_role_permissions(role.permissions)
            || role.permissions.intersects(
                Permissions::MANAGE_MESSAGES
                    | Permissions::MANAGE_THREADS
                    | Permissions::MANAGE_NICKNAMES
                    | Permissions::MANAGE_EVENTS
                    | Permissions::MUTE_MEMBERS
                    | Permissions::DEAFEN_MEMBERS
                    | Permissions::MOVE_MEMBERS
                    | Permissions::VIEW_GUILD_INSIGHTS
                    | Permissions::VIEW_CREATOR_MONETIZATION_ANALYTICS,
            )
        {
            return Err(Denied("On-call role has privileged Discord permissions"));
        }
        Ok(())
    }
}

fn can_manage_roles(guild: &PartialGuild, member: &Member) -> bool {
    let permissions = guild.member_permissions(member);
    if permissions.contains(Permissions::ADMINISTRATOR) {
        return true;
    }
    permissions.contains(Permissions::MANAGE_ROLES)
        && member
            .communication_disabled_until
            .is_none_or(|until| until <= Timestamp::now())
}

fn parse_role_id(role: &str) -> AuthorizationResult<RoleId> {
    validate_snowflake("duty role", role)
        .map(RoleId::new)
        .map_err(|_| Denied("Duty role has an invalid Discord identifier"))
}

#[derive(Deserialize)]
struct RolePermissionMetadata {
    id: RoleId,
    guild_id: Option<GuildId>,
    permissions: String,
    position: i16,
}

fn decode_guild(
    guild_id: GuildId,
    response: Value,
) -> AuthorizationResult<(PartialGuild, HashSet<RoleId>)> {
    let roles = response
        .get("roles")
        .and_then(Value::as_array)
        .context("Discord did not return a full guild role list")?;

    // Serenity overwrites role guild IDs and discards permission bits it does not recognize.
    // Check the raw scope and bits before using its typed guild permission and hierarchy model.
    let mut unknown_permissions = HashSet::new();
    for role in roles {
        let metadata = RolePermissionMetadata::deserialize(role)
            .context("invalid Discord role permission metadata")?;
        if metadata.guild_id.is_some_and(|id| id != guild_id) {
            return Err(anyhow!("Discord returned a role from another guild").into());
        }
        // Serenity maps -1 to zero for @everyone but wraps other negative positions.
        if metadata.position < 0 && (metadata.position != -1 || metadata.id.get() != guild_id.get())
        {
            return Err(anyhow!("Discord returned an invalid role position").into());
        }
        let bits = metadata
            .permissions
            .parse::<u64>()
            .context("invalid Discord role permissions")?;
        if Permissions::from_bits(bits).is_none() {
            unknown_permissions.insert(metadata.id);
        }
    }

    let role_count = roles.len();
    let guild: PartialGuild =
        serde_json::from_value(response).context("invalid Discord guild metadata")?;
    if guild.id != guild_id
        || guild.roles.len() != role_count
        || !guild.roles.contains_key(&RoleId::new(guild_id.get()))
    {
        return Err(anyhow!("Discord returned an inconsistent guild role list").into());
    }

    Ok((guild, unknown_permissions))
}

async fn fetch_member(
    http: &Http,
    guild: &PartialGuild,
    user_id: UserId,
) -> AuthorizationResult<Member> {
    let response: Value = match http
        .fire(Request::new(
            Route::GuildMember {
                guild_id: guild.id,
                user_id,
            },
            LightMethod::Get,
        ))
        .await
    {
        Ok(member) => member,
        Err(serenity::Error::Http(ref error))
            if error.status_code() == Some(StatusCode::NOT_FOUND) =>
        {
            return Err(Denied(
                "Duty authorization member is no longer in this guild",
            ));
        }
        response => response.context("fetch current Discord guild membership")?,
    };

    if let Some(id) = response.get("guild_id")
        && GuildId::deserialize(id).context("invalid Discord member guild ID")? != guild.id
    {
        return Err(anyhow!("Discord returned membership from another guild").into());
    }

    let mut member: Member =
        serde_json::from_value(response).context("invalid Discord guild member metadata")?;
    if member.user.id != user_id {
        return Err(anyhow!("Discord returned a different guild member").into());
    }
    member.guild_id = guild.id;
    validate_member_roles(guild, &member)?;

    Ok(member)
}

fn validate_member_roles(guild: &PartialGuild, member: &Member) -> AuthorizationResult<()> {
    if member
        .roles
        .iter()
        .any(|role| !guild.roles.contains_key(role))
    {
        return Err(
            anyhow!("Discord returned a member role missing from the guild role list").into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn role(id: u64, permissions: u64, position: u16) -> Value {
        json!({
            "id": id.to_string(),
            "name": "Role",
            "permissions": permissions.to_string(),
            "position": position,
            "managed": false,
            "hoist": false,
            "color": 0,
            "colors": { "primary_color": 0 }
        })
    }

    fn guild(roles: Vec<Value>) -> Value {
        json!({
            "id": "1",
            "owner_id": "2",
            "name": "Duty test",
            "verification_level": 0,
            "default_message_notifications": 0,
            "explicit_content_filter": 0,
            "roles": roles,
            "emojis": [],
            "features": [],
            "mfa_level": 0,
            "system_channel_flags": 0,
            "premium_tier": 0,
            "preferred_locale": "en-US",
            "nsfw_level": 0,
            "stickers": [],
            "premium_progress_bar_enabled": false
        })
    }

    fn member(user_id: u64, roles: &[u64]) -> Member {
        let mut member = Member::default();
        member.guild_id = GuildId::new(1);
        member.user.id = UserId::new(user_id);
        member.roles = roles.iter().copied().map(RoleId::new).collect();
        member
    }

    #[test]
    fn missing_role_metadata_and_foreign_roles_fail_closed() {
        let guild_id = GuildId::new(1);
        let mut missing = role(3, 0, 5);
        missing.as_object_mut().unwrap().remove("permissions");
        assert!(matches!(
            decode_guild(guild_id, guild(vec![role(1, 0, 0), missing])),
            Err(DutyAuthorizationError::Unavailable(_))
        ));

        let mut foreign = role(3, 0, 5);
        foreign["guild_id"] = json!("10");
        assert!(matches!(
            decode_guild(guild_id, guild(vec![role(1, 0, 0), foreign])),
            Err(DutyAuthorizationError::Unavailable(_))
        ));

        let mut negative_rank = role(3, 0, 5);
        negative_rank["position"] = json!(-2);
        assert!(matches!(
            decode_guild(guild_id, guild(vec![role(1, 0, 0), negative_rank])),
            Err(DutyAuthorizationError::Unavailable(_))
        ));

        assert!(matches!(
            decode_guild(guild_id, guild(vec![role(3, 0, 5)])),
            Err(DutyAuthorizationError::Unavailable(_))
        ));
    }

    #[test]
    fn unknown_permissions_deny_assignment_and_allow_tracked_cleanup() {
        let (guild, unknown_permissions) = decode_guild(
            GuildId::new(1),
            guild(vec![
                role(1, 0, 0),
                role(3, Permissions::MANAGE_ROLES.bits(), 10),
                role(4, 1 << 63, 9),
                role(5, Permissions::MANAGE_GUILD.bits(), 9),
                role(7, Permissions::MANAGE_MESSAGES.bits(), 9),
                role(8, Permissions::MANAGE_THREADS.bits(), 9),
            ]),
        )
        .unwrap();
        let authorization = DutyRoleAuthorization {
            guild,
            bot: member(6, &[3]),
            unknown_permissions,
        };

        for role_id in [4, 5, 7, 8].map(RoleId::new) {
            assert!(matches!(
                authorization.authorize_assignment(role_id, &member(2, &[])),
                Err(DutyAuthorizationError::Denied(_))
            ));
            assert!(matches!(
                authorization.authorize_removal(role_id, false),
                Err(DutyAuthorizationError::Denied(_))
            ));
            assert_eq!(
                authorization.authorize_removal(role_id, true).unwrap(),
                RemovalAuthorization::Present
            );
        }
        assert_eq!(
            authorization
                .authorize_removal(RoleId::new(6), true)
                .unwrap(),
            RemovalAuthorization::Deleted
        );
    }

    #[test]
    fn current_member_permissions_and_position_ties_control_assignment() {
        let (guild, unknown_permissions) = decode_guild(
            GuildId::new(1),
            guild(vec![
                role(1, 0, 0),
                role(3, Permissions::MANAGE_ROLES.bits(), 10),
                role(4, 0, 10),
                role(5, Permissions::ADMINISTRATOR.bits(), 10),
                role(6, Permissions::MANAGE_GUILD.bits(), 20),
            ]),
        )
        .unwrap();
        let authorization = DutyRoleAuthorization {
            guild,
            bot: member(7, &[3]),
            unknown_permissions,
        };

        let duty_role = RoleId::new(4);
        for actor in [member(2, &[]), member(8, &[3])] {
            assert!(
                authorization
                    .authorize_assignment(duty_role, &actor)
                    .is_ok()
            );
        }
        for actor in [member(8, &[5]), member(8, &[6])] {
            assert!(matches!(
                authorization.authorize_assignment(duty_role, &actor),
                Err(DutyAuthorizationError::Denied(_))
            ));
        }
        assert!(matches!(
            validate_member_roles(&authorization.guild, &member(8, &[42])),
            Err(DutyAuthorizationError::Unavailable(_))
        ));
    }
}
