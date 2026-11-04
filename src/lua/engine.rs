use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use mlua::{
    Error as MluaError, Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, Value,
    VmState,
};
use thiserror::Error;
use tokio::sync::RwLock;

use super::{
    install_xml::{InstallManifest, InstallModuleEntry},
    model::{
        LuaAction, LuaCommandOption, LuaEventContext, LuaExecutionContext, LuaModuleManifest,
        LuaOptionKind,
    },
};

const MAX_ACTIONS_PER_EXECUTION: usize = 25;
const MAX_COMMANDS: usize = 100;
const MAX_OPTIONS_PER_LEVEL: usize = 25;

#[derive(Clone, Copy, Debug)]
pub struct LuaLimits {
    pub memory_bytes: usize,
    pub instruction_limit: i64,
    pub hook_granularity: u32,
}

#[derive(Clone)]
pub struct LuaEngine {
    scripts_dir: Arc<PathBuf>,
    limits: LuaLimits,
    registry: Arc<RwLock<Registry>>,
}

#[derive(Default)]
struct Registry {
    modules: BTreeMap<String, Arc<LoadedModule>>,
    command_to_module: HashMap<String, String>,
    event_to_modules: HashMap<String, Vec<String>>,
    installer_manifest: Option<InstallManifest>,
    default_configs: HashMap<String, serde_json::Value>,
}

#[derive(Clone)]
struct LoadedModule {
    manifest: LuaModuleManifest,
    source: Arc<str>,
    display_name: Arc<str>,
    config_validator: Arc<jsonschema::Validator>,
    #[allow(dead_code)]
    default_config: serde_json::Value,
    #[allow(dead_code)]
    installer_entry: Option<InstallModuleEntry>,
}

#[derive(Debug, Error)]
pub enum LuaEngineError {
    #[error("invalid install.xml at {path}: {message}")]
    InvalidInstallXml { path: PathBuf, message: String },
    #[error("cannot read Lua scripts from {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Lua runtime error in {script}: {source}")]
    Runtime {
        script: String,
        #[source]
        source: MluaError,
    },
    #[error("invalid Lua module {script}: {message}")]
    InvalidModule { script: String, message: String },
    #[error("duplicate Lua module id {0}")]
    DuplicateModule(String),
    #[error("duplicate slash command /{0}")]
    DuplicateCommand(String),
    #[error("invalid config schema in Lua module {script}: {message}")]
    InvalidConfigSchema { script: String, message: String },
    #[error("unknown Lua module {0}")]
    UnknownModuleForConfig(String),
    #[error("unknown Lua module {0}")]
    UnknownModule(String),
    #[error("module {module_id} does not implement command /{command}")]
    UnknownCommand { module_id: String, command: String },
    #[error("module {module_id} does not subscribe to event {event}")]
    UnknownEvent { module_id: String, event: String },
    #[error("Lua execution task failed: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

impl LuaEngine {
    pub fn load(scripts_dir: impl AsRef<Path>, limits: LuaLimits) -> Result<Self, LuaEngineError> {
        let scripts_dir = scripts_dir.as_ref().to_path_buf();
        let registry = build_registry(&scripts_dir, limits)?;

        Ok(Self {
            scripts_dir: Arc::new(scripts_dir),
            limits,
            registry: Arc::new(RwLock::new(registry)),
        })
    }

    pub async fn reload(&self) -> Result<usize, LuaEngineError> {
        let path = Arc::clone(&self.scripts_dir);
        let limits = self.limits;
        let next = tokio::task::spawn_blocking(move || build_registry(&path, limits)).await??;
        let count = next.modules.len();
        *self.registry.write().await = next;
        Ok(count)
    }

    pub async fn manifests(&self) -> Vec<LuaModuleManifest> {
        self.registry
            .read()
            .await
            .modules
            .values()
            .map(|module| module.manifest.clone())
            .collect()
    }

    pub async fn command_definitions(&self) -> Vec<super::model::LuaCommandDefinition> {
        self.registry
            .read()
            .await
            .modules
            .values()
            .flat_map(|module| module.manifest.commands.clone())
            .collect()
    }

    pub async fn module_for_command(&self, command: &str) -> Option<String> {
        self.registry
            .read()
            .await
            .command_to_module
            .get(command)
            .cloned()
    }

    pub async fn modules_for_event(&self, event: &str) -> Vec<String> {
        self.registry
            .read()
            .await
            .event_to_modules
            .get(event)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn manifest(&self, module_id: &str) -> Option<LuaModuleManifest> {
        self.registry
            .read()
            .await
            .modules
            .get(module_id)
            .map(|module| module.manifest.clone())
    }

    pub async fn installer_manifest(&self) -> Option<InstallManifest> {
        self.registry.read().await.installer_manifest.clone()
    }

    pub async fn default_config(&self, module_id: &str) -> serde_json::Value {
        self.registry
            .read()
            .await
            .default_configs
            .get(module_id)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}))
    }

    pub async fn is_module_enabled_in_installer(&self, module_id: &str) -> bool {
        let reg = self.registry.read().await;
        if let Some(installer) = &reg.installer_manifest {
            installer.is_module_enabled(module_id)
        } else {
            true
        }
    }

    pub async fn validate_module_config(
        &self,
        module_id: &str,
        config: &serde_json::Value,
    ) -> Result<(), Vec<String>> {
        let registry = self.registry.read().await;
        let Some(module) = registry.modules.get(module_id) else {
            return Err(vec![format!("Unknown Lua module: {module_id}")]);
        };
        let errors: Vec<String> = module
            .config_validator
            .iter_errors(config)
            .map(|e| {
                let path = e.instance_path().to_string();
                if path.is_empty() {
                    e.to_string()
                } else {
                    format!("{path}: {e}")
                }
            })
            .collect();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub async fn execute_command(
        &self,
        module_id: &str,
        command: &str,
        context: LuaExecutionContext,
    ) -> Result<Vec<LuaAction>, LuaEngineError> {
        let module = self
            .registry
            .read()
            .await
            .modules
            .get(module_id)
            .cloned()
            .ok_or_else(|| LuaEngineError::UnknownModule(module_id.to_owned()))?;

        if !module
            .manifest
            .commands
            .iter()
            .any(|definition| definition.name == command)
        {
            return Err(LuaEngineError::UnknownCommand {
                module_id: module_id.to_owned(),
                command: command.to_owned(),
            });
        }

        let limits = self.limits;
        let command = command.to_owned();
        let module_id = module_id.to_owned();
        tokio::task::spawn_blocking(move || {
            execute_handler(
                &module,
                limits,
                "on_command",
                (&command, &context),
                &module_id,
            )
        })
        .await?
    }

    pub async fn execute_event(
        &self,
        module_id: &str,
        event: &str,
        context: LuaEventContext,
    ) -> Result<Vec<LuaAction>, LuaEngineError> {
        let module = self
            .registry
            .read()
            .await
            .modules
            .get(module_id)
            .cloned()
            .ok_or_else(|| LuaEngineError::UnknownModule(module_id.to_owned()))?;

        if !module.manifest.events.iter().any(|name| name == event) {
            return Err(LuaEngineError::UnknownEvent {
                module_id: module_id.to_owned(),
                event: event.to_owned(),
            });
        }

        let limits = self.limits;
        let event = event.to_owned();
        let module_id = module_id.to_owned();
        tokio::task::spawn_blocking(move || {
            execute_handler(&module, limits, "on_event", (&event, &context), &module_id)
        })
        .await?
    }
}

fn build_registry(scripts_dir: &Path, limits: LuaLimits) -> Result<Registry, LuaEngineError> {
    let install_xml_path = scripts_dir.join("install.xml");
    let installer_manifest = if install_xml_path.is_file() {
        let content =
            fs::read_to_string(&install_xml_path).map_err(|source| LuaEngineError::Io {
                path: install_xml_path.clone(),
                source,
            })?;
        let manifest =
            InstallManifest::parse(&content).map_err(|err| LuaEngineError::InvalidInstallXml {
                path: install_xml_path.clone(),
                message: err.to_string(),
            })?;
        Some(manifest)
    } else {
        None
    };

    let mut registry = Registry {
        installer_manifest: installer_manifest.clone(),
        ..Default::default()
    };

    let candidate_paths: Vec<(PathBuf, Option<InstallModuleEntry>, serde_json::Value)> =
        if let Some(ref installer) = installer_manifest {
            let mut items = Vec::new();
            for module_entry in &installer.modules {
                if !module_entry.enabled {
                    tracing::info!(
                        module_id = %module_entry.id,
                        "skipping module disabled in install.xml"
                    );
                    continue;
                }

                let folder_dir = scripts_dir.join(&module_entry.folder);
                let primary = folder_dir.join(&module_entry.entry);
                let fallback_module = folder_dir.join("module.lua");
                let fallback_named = folder_dir.join(format!("{}.lua", module_entry.id));
                let flat_fallback = scripts_dir.join(format!("{}.lua", module_entry.id));

                let path = if primary.is_file() {
                    primary
                } else if fallback_module.is_file() {
                    fallback_module
                } else if fallback_named.is_file() {
                    fallback_named
                } else if flat_fallback.is_file() {
                    flat_fallback
                } else {
                    return Err(LuaEngineError::InvalidModule {
                        script: primary.display().to_string(),
                        message: format!("module entry file not found for '{}'", module_entry.id),
                    });
                };

                let config_file = module_entry
                    .config
                    .as_ref()
                    .map(|c| folder_dir.join(c))
                    .unwrap_or_else(|| folder_dir.join("config.json"));

                let default_config = if config_file.is_file() {
                    let text =
                        fs::read_to_string(&config_file).map_err(|source| LuaEngineError::Io {
                            path: config_file.clone(),
                            source,
                        })?;
                    serde_json::from_str(&text).map_err(|err| {
                        LuaEngineError::InvalidConfigSchema {
                            script: config_file.display().to_string(),
                            message: format!("invalid JSON syntax: {err}"),
                        }
                    })?
                } else {
                    serde_json::json!({})
                };

                items.push((path, Some(module_entry.clone()), default_config));
            }
            items
        } else {
            let mut paths = Vec::new();
            collect_lua_files(scripts_dir, &mut paths)?;
            paths.sort();
            let mut items = Vec::new();
            for p in paths {
                let config_path = p.parent().map(|dir| dir.join("config.json"));
                let default_config = if let Some(cfg) = config_path {
                    if cfg.is_file() {
                        let text =
                            fs::read_to_string(&cfg).map_err(|source| LuaEngineError::Io {
                                path: cfg.clone(),
                                source,
                            })?;
                        serde_json::from_str(&text).map_err(|err| {
                            LuaEngineError::InvalidConfigSchema {
                                script: cfg.display().to_string(),
                                message: format!("invalid JSON syntax: {err}"),
                            }
                        })?
                    } else {
                        serde_json::json!({})
                    }
                } else {
                    serde_json::json!({})
                };
                items.push((p, None, default_config));
            }
            items
        };

    for (path, installer_entry, default_config) in candidate_paths {
        let source = fs::read_to_string(&path).map_err(|source| LuaEngineError::Io {
            path: path.clone(),
            source,
        })?;
        let display_name = path.display().to_string();
        let mut manifest = inspect_manifest(&source, &display_name, limits)?;
        validate_manifest(&manifest, &display_name)?;

        if let Some(ref entry) = installer_entry {
            if manifest.author.is_none() {
                manifest.author = entry.author.clone();
            }
            if manifest.folder.is_none() {
                manifest.folder = Some(entry.folder.display().to_string());
            }
        }

        if registry.modules.contains_key(&manifest.id) {
            return Err(LuaEngineError::DuplicateModule(manifest.id));
        }

        for command in &manifest.commands {
            if registry
                .command_to_module
                .insert(command.name.clone(), manifest.id.clone())
                .is_some()
            {
                return Err(LuaEngineError::DuplicateCommand(command.name.clone()));
            }
        }

        for event in &manifest.events {
            registry
                .event_to_modules
                .entry(event.clone())
                .or_default()
                .push(manifest.id.clone());
        }

        let config_validator =
            jsonschema::validator_for(&manifest.config_schema).map_err(|error| {
                LuaEngineError::InvalidConfigSchema {
                    script: display_name.clone(),
                    message: error.to_string(),
                }
            })?;

        let errors: Vec<String> = config_validator
            .iter_errors(&default_config)
            .map(|error| format!("{}: {error}", error.instance_path()))
            .collect();
        if !errors.is_empty() {
            return Err(LuaEngineError::InvalidModule {
                script: display_name.clone(),
                message: format!("invalid bundled config: {}", errors.join("; ")),
            });
        }

        registry
            .default_configs
            .insert(manifest.id.clone(), default_config.clone());

        registry.modules.insert(
            manifest.id.clone(),
            Arc::new(LoadedModule {
                manifest,
                source: Arc::from(source),
                display_name: Arc::from(display_name),
                config_validator: Arc::new(config_validator),
                default_config,
                installer_entry,
            }),
        );
    }

    if registry.modules.is_empty() {
        return Err(LuaEngineError::InvalidModule {
            script: scripts_dir.display().to_string(),
            message: "no .lua modules were found".to_owned(),
        });
    }
    if registry.command_to_module.len() > MAX_COMMANDS {
        return Err(LuaEngineError::InvalidModule {
            script: scripts_dir.display().to_string(),
            message: format!(
                "the project defines {} global commands; Discord allows at most {MAX_COMMANDS}",
                registry.command_to_module.len()
            ),
        });
    }

    Ok(registry)
}

fn collect_lua_files(path: &Path, output: &mut Vec<PathBuf>) -> Result<(), LuaEngineError> {
    let entries = fs::read_dir(path).map_err(|source| LuaEngineError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| LuaEngineError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_lua_files(&entry_path, output)?;
        } else if entry_path
            .extension()
            .is_some_and(|extension| extension == "lua")
        {
            output.push(entry_path);
        }
    }

    Ok(())
}

fn inspect_manifest(
    source: &str,
    display_name: &str,
    limits: LuaLimits,
) -> Result<LuaModuleManifest, LuaEngineError> {
    if source.as_bytes().starts_with(b"\x1b") {
        return Err(LuaEngineError::InvalidModule {
            script: display_name.to_owned(),
            message: "precompiled Lua bytecode is not permitted".to_owned(),
        });
    }

    let lua = sandboxed_lua(limits).map_err(|source| LuaEngineError::Runtime {
        script: display_name.to_owned(),
        source,
    })?;
    let module: Table = lua
        .load(source)
        .set_name(display_name)
        .eval()
        .map_err(|source| LuaEngineError::Runtime {
            script: display_name.to_owned(),
            source,
        })?;
    let manifest_value: Value =
        module
            .get("manifest")
            .map_err(|source| LuaEngineError::Runtime {
                script: display_name.to_owned(),
                source,
            })?;

    lua.from_value(manifest_value)
        .map_err(|source| LuaEngineError::Runtime {
            script: display_name.to_owned(),
            source,
        })
}

fn json_to_lua(lua: &Lua, value: serde_json::Value) -> Result<Value, MluaError> {
    match value {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Number(f))
            } else {
                Ok(Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(lua.create_string(&s)?)),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (idx, item) in arr.into_iter().enumerate() {
                table.raw_set(idx + 1, json_to_lua(lua, item)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (key, val) in map {
                if !val.is_null() {
                    table.set(key, json_to_lua(lua, val)?)?;
                }
            }
            Ok(Value::Table(table))
        }
    }
}

fn execute_handler<T: serde::Serialize>(
    module: &LoadedModule,
    limits: LuaLimits,
    handler_name: &str,
    arguments: (&str, &T),
    module_id: &str,
) -> Result<Vec<LuaAction>, LuaEngineError> {
    if module.source.as_bytes().starts_with(b"\x1b") {
        return Err(LuaEngineError::InvalidModule {
            script: module.display_name.to_string(),
            message: "precompiled Lua bytecode is not permitted".to_owned(),
        });
    }

    let lua = sandboxed_lua(limits).map_err(|source| LuaEngineError::Runtime {
        script: module.display_name.to_string(),
        source,
    })?;
    let module_table: Table = lua
        .load(module.source.as_ref())
        .set_name(module.display_name.as_ref())
        .eval()
        .map_err(|source| LuaEngineError::Runtime {
            script: module.display_name.to_string(),
            source,
        })?;
    let handler: Function =
        module_table
            .get(handler_name)
            .map_err(|source| LuaEngineError::Runtime {
                script: module.display_name.to_string(),
                source,
            })?;

    let json_arg = serde_json::to_value(arguments.1).map_err(|err| LuaEngineError::Runtime {
        script: module.display_name.to_string(),
        source: MluaError::external(err),
    })?;
    let context = json_to_lua(&lua, json_arg).map_err(|source| LuaEngineError::Runtime {
        script: module.display_name.to_string(),
        source,
    })?;
    let output: Value =
        handler
            .call((arguments.0, context))
            .map_err(|source| LuaEngineError::Runtime {
                script: module.display_name.to_string(),
                source,
            })?;
    let actions: Vec<LuaAction> =
        lua.from_value(output)
            .map_err(|source| LuaEngineError::Runtime {
                script: module.display_name.to_string(),
                source,
            })?;

    if actions.len() > MAX_ACTIONS_PER_EXECUTION {
        return Err(LuaEngineError::InvalidModule {
            script: module_id.to_owned(),
            message: format!(
                "a script returned {} actions; the limit is {MAX_ACTIONS_PER_EXECUTION}",
                actions.len()
            ),
        });
    }
    for action in &actions {
        action
            .validate()
            .map_err(|message| LuaEngineError::InvalidModule {
                script: module_id.to_owned(),
                message,
            })?;
    }

    Ok(actions)
}

fn sandboxed_lua(limits: LuaLimits) -> mlua::Result<Lua> {
    let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())?;
    lua.set_memory_limit(limits.memory_bytes)?;

    let globals = lua.globals();
    for denied in [
        "collectgarbage",
        "coroutine",
        "debug",
        "dofile",
        "io",
        "load",
        "loadfile",
        "os",
        "package",
        "pcall",
        "require",
        "xpcall",
    ] {
        globals.set(denied, Value::Nil)?;
    }

    let api = lua.create_table()?;
    api.set("api_version", 1)?;
    api.set(
        "escape_mentions",
        lua.create_function(|_, input: String| Ok(input.replace('@', "@\u{200B}")))?,
    )?;
    api.set(
        "truncate",
        lua.create_function(|_, (input, max_chars): (String, usize)| {
            Ok(input.chars().take(max_chars).collect::<String>())
        })?,
    )?;
    api.set(
        "unix_time",
        lua.create_function(|_, ()| {
            use std::time::{SystemTime, UNIX_EPOCH};
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(MluaError::external)?
                .as_secs();
            Ok(seconds)
        })?,
    )?;
    api.set(
        "t",
        lua.create_function(
            |_, (key, arg2, arg3): (String, Option<Value>, Option<Value>)| {
                let mut locale = "en".to_owned();
                let mut args_map: HashMap<String, String> = HashMap::new();

                let parse_arg =
                    |val: Value, loc: &mut String, map: &mut HashMap<String, String>| match val {
                        Value::String(s) => {
                            if let Ok(str_val) = s.to_str() {
                                *loc = str_val.to_owned();
                            }
                        }
                        Value::Table(tbl) => {
                            if let Ok(l) = tbl.get::<String>("locale") {
                                *loc = l;
                            }
                            for pair in tbl.pairs::<Value, Value>().flatten() {
                                if let (Value::String(k), v) = pair
                                    && let Ok(k_str) = k.to_str()
                                    && k_str != "locale"
                                {
                                    let val_str = match v {
                                        Value::String(s) => {
                                            s.to_str().map(|s| s.to_owned()).unwrap_or_default()
                                        }
                                        Value::Integer(i) => i.to_string(),
                                        Value::Number(n) => n.to_string(),
                                        Value::Boolean(b) => b.to_string(),
                                        _ => String::new(),
                                    };
                                    map.insert(k_str.to_owned(), val_str);
                                }
                            }
                        }
                        _ => {}
                    };

                if let Some(a2) = arg2 {
                    parse_arg(a2, &mut locale, &mut args_map);
                }
                if let Some(a3) = arg3 {
                    parse_arg(a3, &mut locale, &mut args_map);
                }

                if args_map.is_empty() {
                    Ok(crate::translations::t(&locale, &key))
                } else {
                    Ok(crate::translations::t_map(&locale, &key, &args_map))
                }
            },
        )?,
    )?;
    api.set("translate", api.get::<Value>("t")?)?;
    globals.set("t", api.get::<Value>("t")?)?;
    globals.set("rustybot", api.clone())?;
    globals.set("zuckerbot", api)?;

    let remaining = AtomicI64::new(limits.instruction_limit);
    let decrement = i64::from(limits.hook_granularity);
    lua.set_global_hook(
        HookTriggers::new().every_nth_instruction(limits.hook_granularity),
        move |_, _| {
            if remaining.fetch_sub(decrement, Ordering::Relaxed) <= decrement {
                Err(MluaError::runtime("Lua instruction limit exceeded"))
            } else {
                Ok(VmState::Continue)
            }
        },
    )?;

    Ok(lua)
}

fn validate_manifest(
    manifest: &LuaModuleManifest,
    display_name: &str,
) -> Result<(), LuaEngineError> {
    validate_identifier(&manifest.id, 64, "module id", display_name)?;
    if manifest.name.trim().is_empty() || manifest.name.chars().count() > 64 {
        return invalid(display_name, "module name must contain 1 to 64 characters");
    }
    if manifest.description.trim().is_empty() || manifest.description.chars().count() > 200 {
        return invalid(
            display_name,
            "module description must contain 1 to 200 characters",
        );
    }
    if manifest.category.trim().is_empty() || manifest.category.chars().count() > 32 {
        return invalid(
            display_name,
            "module category must contain 1 to 32 characters",
        );
    }

    for command in &manifest.commands {
        validate_identifier(&command.name, 32, "command name", display_name)?;
        let description_length = command.description.chars().count();
        if !(1..=100).contains(&description_length) {
            return invalid(
                display_name,
                "command descriptions must contain 1 to 100 characters",
            );
        }
        command
            .validate_contexts()
            .map_err(|message| LuaEngineError::InvalidModule {
                script: display_name.to_owned(),
                message,
            })?;
        validate_options(&command.options, display_name)?;
    }

    for event in &manifest.events {
        validate_identifier(event, 64, "event name", display_name)?;
    }

    Ok(())
}

fn validate_options(
    options: &[LuaCommandOption],
    display_name: &str,
) -> Result<(), LuaEngineError> {
    if options.len() > MAX_OPTIONS_PER_LEVEL {
        return invalid(
            display_name,
            "a command or subcommand cannot define more than 25 options",
        );
    }

    for option in options {
        validate_identifier(&option.name, 32, "option name", display_name)?;
        let description_length = option.description.chars().count();
        if !(1..=100).contains(&description_length) {
            return invalid(
                display_name,
                "option descriptions must contain 1 to 100 characters",
            );
        }
        if option.choices.len() > 25 {
            return invalid(display_name, "an option cannot define more than 25 choices");
        }
        if matches!(
            option.kind,
            LuaOptionKind::Subcommand | LuaOptionKind::SubcommandGroup
        ) {
            validate_options(&option.options, display_name)?;
        } else if !option.options.is_empty() {
            return invalid(
                display_name,
                "only subcommands and subcommand groups can contain nested options",
            );
        }
    }

    Ok(())
}

fn validate_identifier(
    value: &str,
    max_len: usize,
    label: &str,
    display_name: &str,
) -> Result<(), LuaEngineError> {
    let length = value.len();
    let valid = (1..=max_len).contains(&length)
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        });

    if valid {
        Ok(())
    } else {
        invalid(
            display_name,
            &format!(
                "{label} {value:?} must use 1 to {max_len} lowercase ASCII letters, digits, '_' or '-'"
            ),
        )
    }
}

fn invalid<T>(script: &str, message: &str) -> Result<T, LuaEngineError> {
    Err(LuaEngineError::InvalidModule {
        script: script.to_owned(),
        message: message.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rustybot_t_in_lua() {
        let limits = LuaLimits {
            memory_bytes: 2 * 1024 * 1024,
            instruction_limit: 100_000,
            hook_granularity: 100,
        };
        let lua = sandboxed_lua(limits).expect("sandbox init");

        let res_en: String = lua
            .load(r#"return rustybot.t("system.command_executed")"#)
            .eval()
            .expect("eval");
        assert_eq!(res_en, "Command executed successfully.");

        let res_pl: String = lua
            .load(
                r#"
                local ctx = { locale = "pl" }
                return rustybot.t("system.command_executed", ctx)
            "#,
            )
            .eval()
            .expect("eval");
        assert_eq!(res_pl, "Polecenie zostało wykonane.");

        let res_de: String = lua
            .load(
                r#"
                return t("system.rate_limited", { command = "help", seconds = "10" }, "de")
            "#,
            )
            .eval()
            .expect("eval");
        assert_eq!(
            res_de,
            "Ratenbegrenzung für Befehl `/help` überschritten. Bitte in 10 s erneut versuchen."
        );
    }
}
