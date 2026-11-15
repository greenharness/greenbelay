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

use crate::migrate::{Direction, Env, Step};
use crate::names::{self, old};
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
/// marketplace id, when there is one. Read-only; a file that is missing or
/// does not parse reports nothing.
pub fn old_registration(env: &Env) -> Option<PathBuf> {
    let path = env.config_dir.join(SETTINGS_FILE);
    let raw = fs::read_to_string(&path).ok()?;
    let Value::Object(root) = serde_json::from_str::<Value>(&raw).ok()? else {
        return None;
    };
    let (old_side, _) = sides(env, Direction::Forward);
    old_side.registered(&root).then_some(path)
}

/// One file to write, and the line that says so.
struct Edit {
    path: PathBuf,
    after: String,
    line: String,
}

/// `map` with the key `from` renamed to `to` in place, its value passed
/// through `fix`; false when `from` is not there.
fn rename_member(
    root: &mut Map<String, Value>,
    section: &str,
    from: &str,
    to: &str,
    fix: impl Fn(&mut Value),
) -> bool {
    let Some(map) = root.get_mut(section).and_then(Value::as_object_mut) else {
        return false;
    };
    if !map.contains_key(from) {
        return false;
    }
    let rebuilt: Map<String, Value> = std::mem::take(map)
        .into_iter()
        .map(|(key, mut value)| {
            if key == from {
                fix(&mut value);
                (to.to_string(), value)
            } else {
                (key, value)
            }
        })
        .collect();
    *map = rebuilt;
    true
}

/// Replace the string at `pointer` when it is exactly `from`.
fn replace_string(value: &mut Value, pointer: &str, from: &Path, to: &Path) {
    if let Some(slot) = value.pointer_mut(pointer) {
        if slot.as_str() == Some(&from.display().to_string()) {
            *slot = Value::String(to.display().to_string());
        }
    }
}

fn settings_edit(env: &Env, from: &Side, to: &Side) -> Result<Option<Edit>, String> {
    let path = env.config_dir.join(SETTINGS_FILE);
    let Some(raw) = read_file(&path)? else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let parsed: Value = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} is not valid JSON ({e}); fix the file and run again",
            path.display()
        )
    })?;
    let Value::Object(mut root) = parsed else {
        return Ok(None);
    };
    match (from.registered(&root), to.registered(&root)) {
        (true, true) => {
            return Err(format!(
            "{} registers the plugin under both {} and {}; remove one registration and run again",
            path.display(),
            from.plugin_id,
            to.plugin_id
        ))
        }
        (false, _) => return Ok(None),
        (true, false) => {}
    }
    let (from_dir, to_dir) = (from.marketplace_dir(), to.marketplace_dir());
    rename_member(
        &mut root,
        "extraKnownMarketplaces",
        from.marketplace,
        to.marketplace,
        |value| replace_string(value, "/source/path", &from_dir, &to_dir),
    );
    rename_member(
        &mut root,
        "enabledPlugins",
        &from.plugin_id,
        &to.plugin_id,
        |_| {},
    );
    rename_member(
        &mut root,
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
fn marketplace_edit(from: &Side, to: &Side) -> Result<Option<Edit>, String> {
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
    let mut doc: Value = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "{} is not valid JSON ({e}); fix it and run again",
            path.display()
        )
    })?;
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

fn read_file(path: &Path) -> Result<Option<String>, String> {
    match path.symlink_metadata() {
        Err(_) => Ok(None),
        Ok(meta) if meta.is_file() || meta.file_type().is_symlink() => fs::read_to_string(path)
            .map(Some)
            .map_err(|e| format!("cannot read {}: {e}; fix it and run again", path.display())),
        Ok(_) => Err(format!(
            "{} is not a file; fix it and run again",
            path.display()
        )),
    }
}

/// Two spaces unless the file says otherwise, and whether it ended in a
/// newline: the formatting `settings.rs` keeps when it writes the same file.
fn shape(raw: &str) -> (String, bool) {
    let indent = raw
        .lines()
        .find_map(|line| {
            let leading: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            (!leading.is_empty() && line.trim_start().starts_with('"')).then_some(leading)
        })
        .unwrap_or_else(|| "  ".to_string());
    (indent, raw.ends_with('\n'))
}

fn render(doc: &Value, indent: &str, trailing_newline: bool) -> String {
    use serde::Serialize;
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut buf, formatter);
    doc.serialize(&mut serializer)
        .expect("a Value always serialises");
    let mut out = String::from_utf8(buf).expect("serde_json emits UTF-8");
    if trailing_newline {
        out.push('\n');
    }
    out
}

/// The edited document rendered in the file's own style, or a refusal when
/// the file's style cannot be reproduced (writing it back would reformat
/// what the migration was not asked to touch).
fn render_guarded(path: &Path, raw: &str, edited: &Value) -> Result<String, String> {
    let (indent, newline) = shape(raw);
    let original: Value = serde_json::from_str(raw).map_err(|e| {
        format!(
            "{} is not valid JSON ({e}); fix it and run again",
            path.display()
        )
    })?;
    if render(&original, &indent, newline) != raw {
        return Err(format!(
            "{} uses formatting this program cannot reproduce, so writing it back would \
             reformat parts nobody asked to change; normalise the file's formatting (two-space \
             pretty JSON) and run again",
            path.display()
        ));
    }
    Ok(render(edited, &indent, newline))
}

fn edits(env: &Env, direction: Direction) -> Result<Vec<Edit>, String> {
    let (from, to) = sides(env, direction);
    let mut found = Vec::new();
    found.extend(marketplace_edit(&from, &to)?);
    found.extend(settings_edit(env, &from, &to)?);
    Ok(found)
}

fn apply(env: &Env, direction: Direction) -> Result<String, String> {
    let found = edits(env, direction)?;
    for edit in &found {
        crate::atomic::write_atomic(&edit.path, &edit.after).map_err(|e| {
            format!(
                "cannot write {}: {e}; fix it and run again",
                edit.path.display()
            )
        })?;
    }
    Ok(match found.len() {
        0 => String::new(),
        1 => "renamed 1 registration file".to_string(),
        count => format!("renamed {count} registration files"),
    })
}

impl Step for PluginRegistration {
    fn name(&self) -> &str {
        "plugin registration"
    }

    fn forward(&self, env: &Env) -> Result<String, String> {
        apply(env, Direction::Forward)
    }

    fn reverse(&self, env: &Env) -> Result<String, String> {
        apply(env, Direction::Reverse)
    }

    fn plan(&self, env: &Env, direction: Direction) -> Result<Vec<String>, String> {
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

    fn registration(side: &Side, enabled: bool) -> Value {
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
        let original = registration(&old_side(&env), true);
        let path = write_settings(&env, &original);
        let step = PluginRegistration::new();
        step.forward(&env).unwrap();
        let migrated = read_json(&path);
        assert_eq!(migrated, registration(&new_side(&env), true));
        assert!(!migrated.to_string().contains(old::DATA_DIR));
        step.reverse(&env).unwrap();
        assert_eq!(read_json(&path), original);
    }

    #[test]
    fn a_disabled_entry_stays_disabled_and_options_survive() {
        let (_scratch, env) = machine();
        let path = write_settings(&env, &registration(&old_side(&env), false));
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
        let mut doc = registration(&old_side(&env), true);
        doc["enabledPlugins"][new_side(&env).plugin_id] = json!(true);
        let path = write_settings(&env, &doc);
        let marketplace = write_marketplace(&env, &old_side(&env));
        let (settings_before, market_before) =
            (fs::read(&path).unwrap(), fs::read(&marketplace).unwrap());
        let message = PluginRegistration::new().forward(&env).unwrap_err();
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
        let settings = write_settings(&env, &registration(&old_side(&env), true));
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
}
