pub mod ai_mcp;
pub mod gaming_hub;
pub mod github;
pub mod gitlab;
pub mod kick;
pub mod marketplace;
pub mod reddit;
pub mod social_publisher;
pub mod steam;
pub mod telegram;
pub mod twitch;
pub mod vk;
pub mod workflow;
pub mod youtube;

use super::ProviderRegistry;

pub fn build_default_provider_registry() -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(Box::new(steam::SteamAdapter));
    registry.register(Box::new(github::GitHubDevopsAdapter));
    registry.register(Box::new(twitch::TwitchExtendedAdapter));
    registry.register(Box::new(kick::KickAdapter));
    registry.register(Box::new(youtube::YouTubeExtendedAdapter));
    registry.register(Box::new(telegram::TelegramBridgeAdapter));
    registry.register(Box::new(workflow::WorkflowBuilderAdapter));
    registry.register(Box::new(gitlab::GitLabAdapter));
    registry.register(Box::new(vk::VkAdapter));
    registry.register(Box::new(reddit::RedditAdapter));
    registry.register(Box::new(gaming_hub::GamingHubAdapter));
    registry.register(Box::new(social_publisher::SocialPublisherAdapter));
    registry.register(Box::new(ai_mcp::AiMcpAdapter));
    registry.register(Box::new(marketplace::MarketplaceAdapter));
    registry
}
