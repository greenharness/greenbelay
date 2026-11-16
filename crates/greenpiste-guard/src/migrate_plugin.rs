//! The plugin-registration step of the one-time migration (ADR-0068
//! `harness.renames`): the `clear-relay` plugin is registered in the Claude
//! config's `settings.json` under the marketplace id, which is the state
//! directory's name, so the rename moves the id, the enabled-plugin key, the
//! known-marketplace entry and its path, and the plugin's `stateDir` option;
//! the placed `marketplace.json` carries the id as its `name`.
//!
//! Only what the migration owns changes: an entry that is disabled stays
//! disabled, unknown options and unrelated fields are untouched, and a file
//! that registers neither id is left alone. A file that registers both is a
//! conflict and nothing is written.

use crate::migrate::{Direction, Env, Step, StepError};
use crate::names::{self, old};
use crate::settings::{
    installed_plugins, known_marketplaces, render, shape, would_reformat, InstalledPlugins,
    KnownMarketplaces,
};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// The plugin's name; its id is `<plugin>@<marketplace>`.
const PLUGIN: &str = "clear-relay";
const SETTINGS_FILE: &str = "settings.json";
const MARKETPLACE_DIR: &str = "claude-marketplace";
const MARKETPLACE_FILE: &str = ".claude-plugin/marketplace.json";

/// The step: move the plugin's registration between the two marketplace ids.
pub struct PluginRegistration;

impl PluginRegistration {
    pub fn new() -> PluginRegistration {
        PluginRegistration
    }
}

impl Default for PluginRegistration {
    fn default() -> Self {
        PluginRegistration::new()
    }
}

/// One marketplace id and where its state lives.
struct Side {
    marketplace: &'static str,
    plugin_id: String,
    state_dir: PathBuf,
}

impl Side {
    fn new(marketplace: &'static str, state_dir: PathBuf) -> Side {
        Side {
            marketplace,
            plugin_id: format!("{PLUGIN}@{marketplace}"),
            state_dir,
        }
    }

    fn marketplace_dir(&self) -> PathBuf {
        self.state_dir.join(MARKETPLACE_DIR)
    }

    fn registered(&self, root: &Map<String, Value>) -> bool {
        has_member(root, "extraKnownMarketplaces", self.marketplace)
            || has_member(root, "enabledPlugins", &self.plugin_id)
            || has_member(root, "pluginConfigs", &self.plugin_id)
    }
}

fn has_member(root: &Map<String, Value>, section: &str, member: &str) -> bool {
    root.get(section)
        .and_then(Value::as_object)
        .is_some_and(|map| map.contains_key(member))
}

/// (from, to) for `direction`.
fn sides(env: &Env, direction: Direction) -> (Side, Side) {
    let new = Side::new(names::DATA_DIR, env.state_root.clone());
    let old = Side::new(old::DATA_DIR, env.state_root.with_file_name(old::DATA_DIR));
    match direction {
        Direction::Forward => (old, new),
        Direction::Reverse => (new, old),
    }
}

/// The `settings.json` that still registers the plugin under the old
/// marketplace id, when there is one. Read-only; a missing file registers
/// nothing, and one that cannot be read or parsed is an error, never "clean".
pub fn old_registration(env: &Env) -> Result<Option<PathBuf>, StepError> {
    let path = env.config_dir.join(SETTINGS_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(StepError::io("read the plugin settings", source).at(&path)),
    };
    let Value::Object(root) = serde_json::from_str::<Value>(&raw)
        .map_err(|why| StepError::refused(format!("{} does not parse: {why}", path.display())))?
    else {
        return Err(StepError::refused(format!(
            "{} is not a JSON object",
            path.display()
        )));
    };
    let (old_side, _) = sides(env, Direction::Forward);
    Ok(old_side.registered(&root).then_some(path))
}

/// What Claude Code's own plugin registry says about one side's plugin.
/// greenpiste only reads it; the fix is `claude plugin` commands the person runs.
#[derive(Debug, PartialEq, Eq)]
pub enum RegistryState {
    Current,
    Stale(Stale),
    /// A file could not be read: `what` names it by its full path and says
    /// why; `hint` is what the person can do about it.
    Unknown {
        what: String,
        hint: &'static str,
    },
    /// settings.json registers the plugin under neither id.
    NotUsed,
}

/// What differs between the registry and the side settings.json registers.
#[derive(Debug, PartialEq, Eq)]
pub struct Stale {
    marketplace: String,
    plugin_id: String,
    marketplace_dir: PathBuf,
    other_marketplace: String,
    other_plugin_id: String,
    other_plugin_listed: bool,
    other_marketplace_listed: bool,
    missing_marketplace: bool,
    missing_plugin: bool,
}

/// The words that say what is wrong and the commands that fix it.
#[derive(Debug, PartialEq, Eq)]
pub struct RegistryReport {
    /// What Claude Code lists or lacks, without a trailing colon.
    pub what: String,
    pub commands: Vec<String>,
    /// What to do when there is no command to run (an unreadable file).
    pub hint: Option<&'static str>,
}

impl Stale {
    pub fn what(&self) -> String {
        if self.other_plugin_listed {
            format!("still lists {}", self.other_plugin_id)
        } else if self.other_marketplace_listed {
            format!("still lists the marketplace {}", self.other_marketplace)
        } else if self.missing_plugin {
            format!("has not registered {}", self.plugin_id)
        } else {
            format!("has not registered the marketplace {}", self.marketplace)
        }
    }

    /// The `claude plugin` commands that bring the registry to this side, in
    /// the order to run them.
    pub fn commands(&self) -> Vec<String> {
        let mut commands = Vec::new();
        if self.missing_marketplace {
            commands.push(format!(
                "claude plugin marketplace add {}",
                shell_quote(&self.marketplace_dir.display().to_string())
            ));
        }
        if self.missing_plugin {
            commands.push(format!(
                "claude plugin install {}",
                shell_quote(&self.plugin_id)
            ));
        }
        if self.other_plugin_listed {
            commands.push(format!(
                "claude plugin uninstall {}",
                shell_quote(&self.other_plugin_id)
            ));
        }
        if self.other_marketplace_listed {
            commands.push(format!(
                "claude plugin marketplace remove {}",
                shell_quote(&self.other_marketplace)
            ));
        }
        commands
    }
}

impl RegistryState {
    /// What to tell the person; `None` when there is nothing to say.
    pub fn report(&self) -> Option<RegistryReport> {
        match self {
            RegistryState::Current | RegistryState::NotUsed => None,
            RegistryState::Stale(stale) => Some(RegistryReport {
                what: format!("Claude Code's plugin registry {}", stale.what()),
                commands: stale.commands(),
                hint: None,
            }),
            RegistryState::Unknown { what, hint } => Some(RegistryReport {
                what: what.clone(),
                commands: Vec::new(),
                hint: Some(hint),
            }),
        }
    }
}

fn shell_quote(argument: &str) -> String {
    let plain = !argument.is_empty()
        && argument
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain {
        argument.to_string()
    } else {
        format!("'{}'", argument.replace('\'', "'\\''"))
    }
}

fn same_path(registered: &str, wanted: &Path) -> bool {
    match (fs::canonicalize(registered), fs::canonicalize(wanted)) {
        (Ok(left), Ok(right)) => left == right,
        _ => Path::new(registered) == wanted,
    }
}

const REGISTRY_HINT: &str = "check the file, or run claude plugin list";
const SETTINGS_HINT: &str = "fix the file and run again";

fn unreadable_registry(env: &Env, file: &str, why: &str) -> RegistryState {
    RegistryState::Unknown {
        what: format!(
            "could not read Claude Code's plugin registry {}: {why}",
            env.config_dir.join("plugins").join(file).display()
        ),
        hint: REGISTRY_HINT,
    }
}

fn unreadable_settings(path_and_why: String) -> RegistryState {
    RegistryState::Unknown {
        what: format!("could not read {path_and_why}"),
        hint: SETTINGS_HINT,
    }
}

fn classify(env: &Env, side: &Side, other: &Side) -> RegistryState {
    let marketplaces = match known_marketplaces(&env.config_dir) {
        KnownMarketplaces::Missing => None,
        KnownMarketplaces::Unreadable(why) => {
            return unreadable_registry(env, "known_marketplaces.json", &why)
        }
        KnownMarketplaces::Document(doc) => Some(doc),
    };
    let installed = match installed_plugins(&env.config_dir) {
        InstalledPlugins::Missing => None,
        InstalledPlugins::Unreadable(why) => {
            return unreadable_registry(env, "installed_plugins.json", &why)
        }
        InstalledPlugins::Document(doc) => Some(doc),
    };
    let marketplace = |name: &str| marketplaces.as_ref().and_then(|doc| doc.get(name));
    let plugin_listed = |id: &str| {
        installed
            .as_ref()
            .and_then(|doc| doc.get("plugins"))
            .and_then(Value::as_object)
            .is_some_and(|plugins| plugins.contains_key(id))
    };
    let wanted_dir = side.marketplace_dir();
    let marketplace_current = marketplace(side.marketplace)
        .and_then(|entry| entry.pointer("/source/path"))
        .and_then(Value::as_str)
        .is_some_and(|path| same_path(path, &wanted_dir));
    let stale = Stale {
        marketplace: side.marketplace.to_string(),
        plugin_id: side.plugin_id.clone(),
        marketplace_dir: wanted_dir,
        other_marketplace: other.marketplace.to_string(),
        other_plugin_id: other.plugin_id.clone(),
        other_plugin_listed: plugin_listed(&other.plugin_id),
        other_marketplace_listed: marketplace(other.marketplace).is_some(),
        missing_marketplace: !marketplace_current,
        missing_plugin: !plugin_listed(&side.plugin_id),
    };
    if stale.other_plugin_listed
        || stale.other_marketplace_listed
        || stale.missing_marketplace
        || stale.missing_plugin
    {
        RegistryState::Stale(stale)
    } else {
        RegistryState::Current
    }
}

/// The registry judged against the side `direction` ends on, whatever
/// settings.json says (an install preview, which is about to register it).
/// Reads only the registry files.
pub fn registry_state_for(env: &Env, direction: Direction) -> RegistryState {
    let (other, side) = sides(env, direction);
    classify(env, &side, &other)
}

/// The registry judged against the side a migration in `direction` leaves
/// settings.json on (a dry run): `NotUsed` when settings.json registers the
/// plugin under neither id, as the run that follows would say. Reads only.
pub fn registry_state_after(env: &Env, direction: Direction) -> RegistryState {
    match registered_sides(env) {
        Registered::Old | Registered::New => registry_state_for(env, direction),
        Registered::Neither => RegistryState::NotUsed,
        Registered::Unreadable(what) => unreadable_settings(what),
    }
}

/// The registry judged against the side settings.json registers now. Reads
/// only; `NotUsed` when it registers neither.
pub fn registry_state(env: &Env) -> RegistryState {
    let (old, new) = sides(env, Direction::Forward);
    match registered_sides(env) {
        Registered::New => classify(env, &new, &old),
        Registered::Old => classify(env, &old, &new),
        Registered::Neither => RegistryState::NotUsed,
        Registered::Unreadable(what) => unreadable_settings(what),
    }
}

/// Which id settings.json registers the plugin under (the new one when both).
enum Registered {
    Old,
    New,
    /// Missing, empty, or registering neither id.
    Neither,
    /// settings.json cannot be read; the text names it and says why.
    Unreadable(String),
}

fn registered_sides(env: &Env) -> Registered {
    let path = env.config_dir.join(SETTINGS_FILE);
    let unreadable = |why: &str| Registered::Unreadable(format!("{}: {why}", path.display()));
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Registered::Neither,
        Err(source) => return unreadable(&source.to_string()),
    };
    if raw.trim().is_empty() {
        return Registered::Neither;
    }
    let root = match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(root)) => root,
        Ok(_) => return unreadable("not a JSON object"),
        Err(why) => return unreadable(&format!("does not parse: {why}")),
    };
    let (old, new) = sides(env, Direction::Forward);
    if new.registered(&root) {
        Registered::New
    } else if old.registered(&root) {
        Registered::Old
    } else {
        Registered::Neither
    }
}

/// One file to write, and the line that says so.
struct Edit {
    path: PathBuf,
    after: String,
    line: String,
}

/// `root` with the key `from` of `section` renamed to `to` in place, its value
/// passed through `fix`; `root` unchanged when `from` is not there.
fn rename_member(
    root: Map<String, Value>,
    section: &str,
    from: &str,
    to: &str,
    fix: impl Fn(Value) -> Value,
) -> Map<String, Value> {
    root.into_iter()
        .map(|(name, value)| match value {
            Value::Object(members) if name == section => {
                let renamed: Map<String, Value> = members
                    .into_iter()
                    .map(|(key, member)| {
                        if key == from {
                            (to.to_string(), fix(member))
                        } else {
                            (key, member)
                        }
                    })
                    .collect();
                (name, Value::Object(renamed))
            }
            other => (name, other),
        })
        .collect()
}

/// `value` with the string at `pointer` replaced when it is exactly `from`.
fn replace_string(mut value: Value, pointer: &str, from: &Path, to: &Path) -> Value {
    if let Some(slot) = value.pointer_mut(pointer) {
        if slot.as_str() == Some(&from.display().to_string()) {
            *slot = Value::String(to.display().to_string());
        }
    }
    value
}

fn invalid_json(path: &Path, error: &serde_json::Error, fix: &str) -> StepError {
    StepError::refused(format!(
        "{} is not valid JSON ({error}); {fix} and run again",
        path.display()
    ))
    .at(path)
}

fn settings_edit(env: &Env, from: &Side, to: &Side) -> Result<Option<Edit>, StepError> {
    let path = env.config_dir.join(SETTINGS_FILE);
    let Some(raw) = read_file(&path)? else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let parsed: Value =
        serde_json::from_str(&raw).map_err(|e| invalid_json(&path, &e, "fix the file"))?;
    let Value::Object(root) = parsed else {
        return Ok(None);
    };
    match (from.registered(&root), to.registered(&root)) {
        (true, true) => {
            return Err(StepError::refused(format!(
            "{} registers the plugin under both {} and {}; remove one registration and run again",
            path.display(),
            from.plugin_id,
            to.plugin_id
        ))
            .at(&path))
        }
        (false, _) => return Ok(None),
        (true, false) => {}
    }
    let (from_dir, to_dir) = (from.marketplace_dir(), to.marketplace_dir());
    let root = rename_member(
        root,
        "extraKnownMarketplaces",
        from.marketplace,
        to.marketplace,
        |value| replace_string(value, "/source/path", &from_dir, &to_dir),
    );
    let root = rename_member(
        root,
        "enabledPlugins",
        &from.plugin_id,
        &to.plugin_id,
        |value| value,
    );
    let root = rename_member(
        root,
        "pluginConfigs",
        &from.plugin_id,
        &to.plugin_id,
        |value| replace_string(value, "/options/stateDir", &from.state_dir, &to.state_dir),
    );
    let after = render_guarded(&path, &raw, &Value::Object(root))?;
    Ok(Some(Edit {
        line: format!(
            "rename {} to {} in {}",
            from.plugin_id,
            to.plugin_id,
            path.display()
        ),
        path,
        after,
    }))
}

/// The placed marketplace file lives under whichever state root exists: the
/// state step may or may not have moved it yet.
fn marketplace_edit(from: &Side, to: &Side) -> Result<Option<Edit>, StepError> {
    let located = [&to.state_dir, &from.state_dir]
        .into_iter()
        .map(|state| state.join(MARKETPLACE_DIR).join(MARKETPLACE_FILE))
        .find(|path| path.is_file());
    let Some(path) = located else {
        return Ok(None);
    };
    let Some(raw) = read_file(&path)? else {
        return Ok(None);
    };
    let mut doc: Value =
        serde_json::from_str(&raw).map_err(|e| invalid_json(&path, &e, "fix it"))?;
    match doc.get_mut("name") {
        Some(name) if name.as_str() == Some(from.marketplace) => {
            *name = Value::String(to.marketplace.to_string());
        }
        _ => return Ok(None),
    }
    let after = render_guarded(&path, &raw, &doc)?;
    Ok(Some(Edit {
        line: format!(
            "set the marketplace name {} to {} in {}",
            from.marketplace,
            to.marketplace,
            path.display()
        ),
        path,
        after,
    }))
}

fn read_file(path: &Path) -> Result<Option<String>, StepError> {
    match path.symlink_metadata() {
        Err(_) => Ok(None),
        Ok(meta) if meta.is_file() || meta.file_type().is_symlink() => fs::read_to_string(path)
            .map(Some)
            .map_err(|e| StepError::io(format!("cannot read {}", path.display()), e).at(path)),
        Ok(_) => Err(StepError::refused(format!(
            "{} is not a file; fix it and run again",
            path.display()
        ))
        .at(path)),
    }
}

/// The edited document rendered in the file's own style, or a refusal when
/// the file's style cannot be reproduced (writing it back would reformat
/// what the migration was not asked to touch).
fn render_guarded(path: &Path, raw: &str, edited: &Value) -> Result<String, StepError> {
    let (indent, newline) = shape(raw);
    let original: Value =
        serde_json::from_str(raw).map_err(|e| invalid_json(path, &e, "fix the file"))?;
    if would_reformat(raw, &original, &indent, newline) {
        return Err(StepError::refused(format!(
            "{} uses formatting this program cannot reproduce, so writing it back would \
             reformat parts nobody asked to change; normalise the file's formatting (two-space \
             pretty JSON) and run again",
            path.display()
        ))
        .at(path));
    }
    Ok(render(edited, &indent, newline))
}

fn edits(env: &Env, direction: Direction) -> Result<Vec<Edit>, StepError> {
    let (from, to) = sides(env, direction);
    let mut found = Vec::new();
    found.extend(marketplace_edit(&from, &to)?);
    found.extend(settings_edit(env, &from, &to)?);
    Ok(found)
}

fn apply(env: &Env, direction: Direction) -> Result<String, StepError> {
    let found = edits(env, direction)?;
    for edit in &found {
        crate::atomic::write_atomic(&edit.path, &edit.after).map_err(|e| {
            StepError::io(format!("cannot write {}", edit.path.display()), e).at(&edit.path)
        })?;
    }
    Ok(match found.len() {
        0 => String::new(),
        1 => "renamed 1 registration file".to_string(),
        count => format!("renamed {count} registration files"),
    })
}

impl Step for PluginRegistration {
    fn name(&self) -> &'static str {
        "plugin registration"
    }

    fn forward(&self, env: &Env) -> Result<String, StepError> {
        apply(env, Direction::Forward)
    }

    fn reverse(&self, env: &Env) -> Result<String, StepError> {
        apply(env, Direction::Reverse)
    }

    fn plan(&self, env: &Env, direction: Direction) -> Result<Vec<String>, StepError> {
        Ok(edits(env, direction)?
            .into_iter()
            .map(|edit| edit.line)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0); // best-effort cleanup of a temp dir
        }
    }

    fn machine() -> (Scratch, Env) {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("gp-plugin-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&root); // a leftover of a crashed run
        fs::create_dir_all(root.join("claude")).unwrap();
        let env = Env {
            config_dir: root.join("claude"),
            state_root: root.join("state").join(names::DATA_DIR),
            git_env: None,
        };
        (Scratch(root), env)
    }

    fn old_side(env: &Env) -> Side {
        sides(env, Direction::Forward).0
    }

    fn new_side(env: &Env) -> Side {
        sides(env, Direction::Forward).1
    }

    fn registration(side: &Side, enabled: Value) -> Value {
        json!({
            "model": "opus",
            "extraKnownMarketplaces": {
                "other": { "source": { "source": "directory", "path": "/elsewhere" } },
                side.marketplace: { "source": { "source": "directory",
                    "path": side.marketplace_dir().display().to_string() } }
            },
            "enabledPlugins": { "unrelated@market": true, side.plugin_id.clone(): enabled },
            "pluginConfigs": {
                side.plugin_id.clone(): { "options": {
                    "stateDir": side.state_dir.display().to_string(),
                    "custom": [1, 2]
                } }
            }
        })
    }

    fn write_settings(env: &Env, doc: &Value) -> PathBuf {
        let path = env.config_dir.join(SETTINGS_FILE);
        fs::write(&path, render(doc, "  ", true)).unwrap();
        path
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn write_marketplace(env: &Env, side: &Side) -> PathBuf {
        let path = side.marketplace_dir().join(MARKETPLACE_FILE);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let doc = json!({ "name": side.marketplace, "owner": { "name": "x" }, "plugins": [] });
        fs::write(&path, render(&doc, "  ", true)).unwrap();
        let _ = env;
        path
    }

    #[test]
    fn forward_then_reverse_equals_the_original() {
        let (_scratch, env) = machine();
        let original = registration(&old_side(&env), json!(true));
        let path = write_settings(&env, &original);
        let step = PluginRegistration::new();
        step.forward(&env).unwrap();
        let migrated = read_json(&path);
        assert_eq!(migrated, registration(&new_side(&env), json!(true)));
        assert!(!migrated.to_string().contains(old::DATA_DIR));
        step.reverse(&env).unwrap();
        assert_eq!(read_json(&path), original);
    }

    #[test]
    fn a_disabled_entry_stays_disabled_and_options_survive() {
        let (_scratch, env) = machine();
        let path = write_settings(&env, &registration(&old_side(&env), json!(false)));
        PluginRegistration::new().forward(&env).unwrap();
        let migrated = read_json(&path);
        let id = new_side(&env).plugin_id;
        assert_eq!(migrated["enabledPlugins"][&id], json!(false));
        assert_eq!(
            migrated["pluginConfigs"][&id]["options"]["custom"],
            json!([1, 2])
        );
        assert_eq!(migrated["model"], json!("opus"));
        assert_eq!(migrated["enabledPlugins"]["unrelated@market"], json!(true));
    }

    #[test]
    fn an_absent_plugin_stays_absent() {
        let (_scratch, env) = machine();
        let path = write_settings(&env, &json!({ "model": "opus" }));
        let before = fs::read(&path).unwrap();
        let step = PluginRegistration::new();
        assert_eq!(step.forward(&env).unwrap(), "");
        assert_eq!(step.reverse(&env).unwrap(), "");
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_file(&path).unwrap();
        step.forward(&env).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn both_registrations_are_a_conflict_that_writes_nothing() {
        let (_scratch, env) = machine();
        let mut doc = registration(&old_side(&env), json!(true));
        doc["enabledPlugins"][new_side(&env).plugin_id] = json!(true);
        let path = write_settings(&env, &doc);
        let marketplace = write_marketplace(&env, &old_side(&env));
        let (settings_before, market_before) =
            (fs::read(&path).unwrap(), fs::read(&marketplace).unwrap());
        let message = PluginRegistration::new()
            .forward(&env)
            .unwrap_err()
            .to_string();
        assert!(message.contains(&old_side(&env).plugin_id), "{message}");
        assert!(message.contains(&new_side(&env).plugin_id), "{message}");
        assert_eq!(fs::read(&path).unwrap(), settings_before);
        assert_eq!(fs::read(&marketplace).unwrap(), market_before);
    }

    #[test]
    fn the_marketplace_name_flips_in_each_direction() {
        let (_scratch, env) = machine();
        let path = write_marketplace(&env, &old_side(&env));
        let step = PluginRegistration::new();
        step.forward(&env).unwrap();
        assert_eq!(read_json(&path)["name"], json!(names::DATA_DIR));
        step.reverse(&env).unwrap();
        assert_eq!(read_json(&path)["name"], json!(old::DATA_DIR));
        assert_eq!(read_json(&path)["owner"]["name"], json!("x"));
    }

    #[test]
    fn plan_reads_only_and_names_what_would_change() {
        let (_scratch, env) = machine();
        let settings = write_settings(&env, &registration(&old_side(&env), json!(true)));
        let marketplace = write_marketplace(&env, &old_side(&env));
        let before = (
            fs::read(&settings).unwrap(),
            fs::read(&marketplace).unwrap(),
        );
        let times = (
            fs::metadata(&settings).unwrap().modified().unwrap(),
            fs::metadata(&marketplace).unwrap().modified().unwrap(),
        );
        let step = PluginRegistration::new();
        assert_eq!(step.plan(&env, Direction::Forward).unwrap().len(), 2);
        assert!(step.plan(&env, Direction::Reverse).unwrap().is_empty());
        assert_eq!(
            (
                fs::read(&settings).unwrap(),
                fs::read(&marketplace).unwrap()
            ),
            before
        );
        assert_eq!(
            (
                fs::metadata(&settings).unwrap().modified().unwrap(),
                fs::metadata(&marketplace).unwrap().modified().unwrap()
            ),
            times
        );
    }

    #[test]
    fn an_unreproducible_format_is_refused_untouched() {
        let (_scratch, env) = machine();
        let path = env.config_dir.join(SETTINGS_FILE);
        let raw = format!(
            "{{\"enabledPlugins\":{{\"{}\":true}}}}",
            old_side(&env).plugin_id
        );
        fs::write(&path, &raw).unwrap();
        assert!(PluginRegistration::new().forward(&env).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), raw);
    }

    fn write_registry(env: &Env, marketplace_entries: &[(&str, &Path)], plugin_ids: &[&str]) {
        let plugins = env.config_dir.join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        let mut known = serde_json::Map::new();
        for (name, dir) in marketplace_entries {
            let dir = dir.display().to_string();
            known.insert(
                name.to_string(),
                json!({ "source": { "source": "directory", "path": dir },
                    "installLocation": dir, "lastUpdated": "2026-11-15T10:00:00.000Z" }),
            );
        }
        let mut installed = serde_json::Map::new();
        for id in plugin_ids {
            installed.insert(
                id.to_string(),
                json!([{ "scope": "user", "installPath": "/cache/x", "version": "1.0.0" }]),
            );
        }
        fs::write(
            plugins.join("known_marketplaces.json"),
            Value::Object(known).to_string(),
        )
        .unwrap();
        fs::write(
            plugins.join("installed_plugins.json"),
            json!({ "version": 2, "plugins": installed }).to_string(),
        )
        .unwrap();
    }

    fn commands(state: RegistryState) -> Vec<String> {
        match state {
            RegistryState::Stale(stale) => stale.commands(),
            other => panic!("expected stale, got {other:?}"),
        }
    }

    fn both_dirs(env: &Env) -> (PathBuf, PathBuf) {
        (
            old_side(env).marketplace_dir(),
            new_side(env).marketplace_dir(),
        )
    }

    #[test]
    fn a_registry_on_the_side_is_current_in_both_directions() {
        let (_scratch, env) = machine();
        let (old_dir, new_dir) = both_dirs(&env);
        write_registry(
            &env,
            &[(names::DATA_DIR, &new_dir)],
            &[&new_side(&env).plugin_id],
        );
        assert_eq!(
            registry_state_for(&env, Direction::Forward),
            RegistryState::Current
        );
        write_registry(
            &env,
            &[(old::DATA_DIR, &old_dir)],
            &[&old_side(&env).plugin_id],
        );
        assert_eq!(
            registry_state_for(&env, Direction::Reverse),
            RegistryState::Current
        );
    }

    #[test]
    fn the_old_registry_gives_the_four_commands_in_order() {
        let (_scratch, env) = machine();
        let (old_dir, new_dir) = both_dirs(&env);
        write_registry(
            &env,
            &[(old::DATA_DIR, &old_dir)],
            &[&old_side(&env).plugin_id],
        );
        assert_eq!(
            commands(registry_state_for(&env, Direction::Forward)),
            vec![
                format!("claude plugin marketplace add {}", new_dir.display()),
                format!("claude plugin install {}", new_side(&env).plugin_id),
                format!("claude plugin uninstall {}", old_side(&env).plugin_id),
                format!("claude plugin marketplace remove {}", old::DATA_DIR),
            ]
        );
        write_registry(
            &env,
            &[(names::DATA_DIR, &new_dir)],
            &[&new_side(&env).plugin_id],
        );
        assert_eq!(
            commands(registry_state_for(&env, Direction::Reverse)),
            vec![
                format!("claude plugin marketplace add {}", old_dir.display()),
                format!("claude plugin install {}", old_side(&env).plugin_id),
                format!("claude plugin uninstall {}", new_side(&env).plugin_id),
                format!("claude plugin marketplace remove {}", names::DATA_DIR),
            ]
        );
    }

    #[test]
    fn each_leftover_or_gap_is_named_alone() {
        let (_scratch, env) = machine();
        let (old_dir, new_dir) = both_dirs(&env);
        let new_id = new_side(&env).plugin_id;
        let old_id = old_side(&env).plugin_id;
        write_registry(&env, &[(names::DATA_DIR, &new_dir)], &[&new_id, &old_id]);
        assert_eq!(
            commands(registry_state_for(&env, Direction::Forward)),
            vec![format!("claude plugin uninstall {old_id}")]
        );
        write_registry(
            &env,
            &[(names::DATA_DIR, &new_dir), (old::DATA_DIR, &old_dir)],
            &[&new_id],
        );
        assert_eq!(
            commands(registry_state_for(&env, Direction::Forward)),
            vec![format!(
                "claude plugin marketplace remove {}",
                old::DATA_DIR
            )]
        );
        write_registry(&env, &[(names::DATA_DIR, &new_dir)], &[]);
        assert_eq!(
            commands(registry_state_for(&env, Direction::Forward)),
            vec![format!("claude plugin install {new_id}")]
        );
        write_registry(&env, &[], &[&new_id]);
        assert_eq!(
            commands(registry_state_for(&env, Direction::Forward)),
            vec![format!(
                "claude plugin marketplace add {}",
                new_dir.display()
            )]
        );
    }

    #[test]
    fn a_missing_file_counts_as_missing_pieces_and_an_unreadable_one_is_unknown() {
        let (_scratch, env) = machine();
        assert_eq!(
            commands(registry_state_for(&env, Direction::Forward)).len(),
            2
        );
        let plugins = env.config_dir.join("plugins");
        for file in ["installed_plugins.json", "known_marketplaces.json"] {
            fs::create_dir_all(plugins.join(file)).unwrap();
            match registry_state_for(&env, Direction::Forward) {
                RegistryState::Unknown { what, hint } => {
                    assert!(what.contains(&format!("/{file}: ")), "{what}");
                    assert_eq!(hint, REGISTRY_HINT);
                }
                other => panic!("{other:?}"),
            }
            fs::remove_dir(plugins.join(file)).unwrap();
        }
    }

    #[test]
    fn settings_decides_the_observed_side() {
        let (_scratch, env) = machine();
        assert_eq!(registry_state(&env), RegistryState::NotUsed);
        write_settings(&env, &json!({ "model": "opus" }));
        assert_eq!(registry_state(&env), RegistryState::NotUsed);
        write_settings(&env, &registration(&new_side(&env), json!(true)));
        assert!(matches!(registry_state(&env), RegistryState::Stale(_)));
        let (_, new_dir) = both_dirs(&env);
        write_registry(
            &env,
            &[(names::DATA_DIR, &new_dir)],
            &[&new_side(&env).plugin_id],
        );
        assert_eq!(registry_state(&env), RegistryState::Current);
        fs::write(env.config_dir.join(SETTINGS_FILE), "{nope").unwrap();
        assert!(matches!(
            registry_state(&env),
            RegistryState::Unknown { .. }
        ));
        fs::remove_file(env.config_dir.join(SETTINGS_FILE)).unwrap();
        fs::create_dir(env.config_dir.join(SETTINGS_FILE)).unwrap();
        assert!(matches!(
            registry_state(&env),
            RegistryState::Unknown { .. }
        ));
    }

    #[test]
    fn a_dry_run_says_nothing_when_settings_registers_no_plugin() {
        let (_scratch, env) = machine();
        assert_eq!(
            registry_state_after(&env, Direction::Forward),
            RegistryState::NotUsed
        );
        write_settings(&env, &json!({ "model": "opus" }));
        assert_eq!(
            registry_state_after(&env, Direction::Reverse),
            RegistryState::NotUsed
        );
        write_settings(&env, &registration(&old_side(&env), json!(true)));
        assert_eq!(
            commands(registry_state_after(&env, Direction::Forward)).len(),
            2
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_marketplace_added_through_a_symlink_is_current() {
        let (scratch, env) = machine();
        let (_, new_dir) = both_dirs(&env);
        fs::create_dir_all(&new_dir).unwrap();
        let link = scratch.0.join("link");
        std::os::unix::fs::symlink(&new_dir, &link).unwrap();
        write_registry(
            &env,
            &[(names::DATA_DIR, &link)],
            &[&new_side(&env).plugin_id],
        );
        assert_eq!(
            registry_state_for(&env, Direction::Forward),
            RegistryState::Current
        );
    }

    #[test]
    fn a_path_with_a_space_or_metacharacter_is_single_quoted() {
        assert_eq!(shell_quote("/a b/c"), "'/a b/c'");
        assert_eq!(shell_quote("/a'b"), "'/a'\\''b'");
        assert_eq!(shell_quote("a;b"), "'a;b'");
        assert_eq!(shell_quote("clear-relay@x"), "clear-relay@x");
        let (_scratch, mut env) = machine();
        env.state_root = env
            .state_root
            .with_file_name("my state")
            .join(names::DATA_DIR);
        let listed = commands(registry_state_for(&env, Direction::Forward));
        assert!(listed[0].contains("add '"), "{listed:?}");
        assert!(
            listed[0].ends_with("my state/greenpiste/claude-marketplace'"),
            "{listed:?}"
        );
    }
}
