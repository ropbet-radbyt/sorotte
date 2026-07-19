use mlua::{Function, Lua, LuaSerdeExt, Table, Value};
use serde_json::{Value as JsonValue, json};

const PROTOCOL: &str = "sorotte-network-options-v3";
const CONFIGURE_MESSAGE: &str = "sorotte_network_options_configure";
const HEARTBEAT_MESSAGE: &str = "sorotte_network_options_heartbeat";
const RELEASE_MESSAGE: &str = "sorotte_network_options_release";
const APPLY_ACTIVE_MESSAGE: &str = "sorotte_network_options_apply_active";
const SCRIPT_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../resources/sorotte_network_options.lua"
));

const MP_MOCK_SOURCE: &str = r#"
__harness = {
    messages = {}, hooks = {}, timers = {}, properties = {}, writes = {}, emissions = {},
    reject = nil, time = 0,
}
mp = { keep_running = true }
function mp.get_script_name() return __script_name end
function mp.register_script_message(name, callback) __harness.messages[name] = callback end
function mp.add_hook(name, priority, callback) __harness.hooks[name] = callback end
function mp.add_periodic_timer(interval, callback)
    table.insert(__harness.timers, { interval = interval, callback = callback })
    return { kill = function() end }
end
function mp.get_time() return __harness.time end
function mp.get_property(name, default)
    local value = __harness.properties[name]
    if value == nil then return default end
    return value
end
function mp.set_property(name, value)
    table.insert(__harness.writes, { name = name, value = value, path = __harness.properties.path })
    if __harness.reject == name then return nil, 'rejected ' .. name end
    return true
end
function mp.commandv(...)
    local args = {...}
    if args[1] == 'script-message' then
        table.insert(__harness.emissions, { name = args[2], payload = args[3] })
    end
end
package.preload['mp.utils'] = function()
    return { parse_json = __parse_json, format_json = __format_json }
end
"#;

struct Harness {
    lua: Lua,
}

impl Harness {
    fn new() -> mlua::Result<Self> {
        let lua = Lua::new();
        let parse_json = lua.create_function(|lua, input: String| {
            let value: JsonValue = serde_json::from_str(&input).map_err(mlua::Error::external)?;
            lua.to_value(&value)
        })?;
        let format_json = lua.create_function(|lua, input: Value| {
            let value: JsonValue = lua.from_value(input)?;
            serde_json::to_string(&value).map_err(mlua::Error::external)
        })?;
        lua.globals().set("__parse_json", parse_json)?;
        lua.globals().set("__format_json", format_json)?;
        lua.globals()
            .set("__script_name", "sorotte_network_options")?;
        lua.load(MP_MOCK_SOURCE).exec()?;
        lua.load(SCRIPT_SOURCE).exec()?;
        Ok(Self { lua })
    }

    fn table(&self) -> mlua::Result<Table> {
        self.lua.globals().get("__harness")
    }

    fn send(&self, name: &str, payload: JsonValue) -> mlua::Result<()> {
        let messages: Table = self.table()?.get("messages")?;
        let callback: Function = messages.get(name)?;
        callback.call(serde_json::to_string(&payload).unwrap())
    }

    fn configure_as(
        &self,
        owner: &str,
        attachment: &str,
        generation: u64,
        options: JsonValue,
    ) -> mlua::Result<()> {
        self.send(
            CONFIGURE_MESSAGE,
            json!({
                "protocol": PROTOCOL,
                "ownerId": owner,
                "attachmentId": attachment,
                "configurationGeneration": generation,
                "leaseMs": 2_000,
                "options": options,
            }),
        )
    }

    fn controller_payload(owner: &str, attachment: &str, generation: u64) -> JsonValue {
        json!({
            "protocol": PROTOCOL,
            "ownerId": owner,
            "attachmentId": attachment,
            "configurationGeneration": generation,
            "heartbeatNonce": 1,
        })
    }

    fn set_path(&self, property: &str, path: &str) -> mlua::Result<()> {
        let properties: Table = self.table()?.get("properties")?;
        properties.set(property, path)
    }

    fn set_rejected_property(&self, property: Option<&str>) -> mlua::Result<()> {
        self.table()?.set("reject", property)
    }

    fn invoke_on_load(&self) -> mlua::Result<()> {
        let hooks: Table = self.table()?.get("hooks")?;
        let callback: Function = hooks.get("on_load")?;
        callback.call(())
    }

    fn advance(&self, seconds: f64) -> mlua::Result<()> {
        let table = self.table()?;
        let now: f64 = table.get("time")?;
        table.set("time", now + seconds)?;
        let timers: Table = table.get("timers")?;
        for timer in timers.sequence_values::<Table>() {
            timer?.get::<Function>("callback")?.call::<()>(())?;
        }
        Ok(())
    }

    fn writes(&self) -> mlua::Result<Vec<(String, String, String)>> {
        let writes: Table = self.table()?.get("writes")?;
        writes
            .sequence_values::<Table>()
            .map(|write| {
                let write = write?;
                Ok((write.get("name")?, write.get("value")?, write.get("path")?))
            })
            .collect()
    }

    fn emissions(&self, name: &str) -> mlua::Result<Vec<JsonValue>> {
        let emissions: Table = self.table()?.get("emissions")?;
        emissions
            .sequence_values::<Table>()
            .filter_map(|emission| match emission {
                Ok(emission) if emission.get::<String>("name").ok().as_deref() == Some(name) => {
                    Some(emission.get::<String>("payload").map(|payload| {
                        serde_json::from_str(&payload).expect("hook payload should be valid JSON")
                    }))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }
}

#[test]
fn configure_heartbeat_release_and_expiry_bound_network_writes() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as("owner-a", "attachment-a", 7, json!({"cache-secs": "75"}))?;
    assert_eq!(
        harness.emissions("sorotte-network-options-configured")?[0]["status"],
        "configured"
    );

    harness.advance(1.5)?;
    harness.send(
        HEARTBEAT_MESSAGE,
        Harness::controller_payload("owner-a", "attachment-a", 7),
    )?;
    let heartbeat = harness.emissions("sorotte-network-options-heartbeat")?;
    assert_eq!(heartbeat[0]["status"], "renewed");
    assert_eq!(heartbeat[0]["heartbeatNonce"], 1);
    harness.advance(1.5)?;
    harness.set_path("path", "https://media.example.test/live-a.m3u8")?;
    harness.set_path(
        "stream-open-filename",
        "https://media.example.test/live-a.m3u8",
    )?;
    harness.invoke_on_load()?;
    assert_eq!(harness.writes()?.len(), 1, "heartbeat must renew the lease");

    harness.send(
        RELEASE_MESSAGE,
        Harness::controller_payload("owner-a", "attachment-a", 7),
    )?;
    harness.set_path("path", "https://media.example.test/after-release.m3u8")?;
    harness.set_path(
        "stream-open-filename",
        "https://media.example.test/after-release.m3u8",
    )?;
    harness.invoke_on_load()?;
    assert_eq!(
        harness.writes()?.len(),
        1,
        "release must clear the policy map"
    );

    harness.configure_as("owner-a", "attachment-b", 8, json!({"cache-secs": "90"}))?;
    harness.advance(2.1)?;
    harness.invoke_on_load()?;
    assert_eq!(
        harness.writes()?.len(),
        1,
        "expiry must clear the policy map"
    );
    assert_eq!(
        harness
            .emissions("sorotte-network-options-ownership")?
            .last()
            .unwrap()["status"],
        "lease-expired"
    );
    Ok(())
}

#[test]
fn configuration_reports_stable_instance_and_monotonic_load_sequence() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as("owner-a", "attachment-a", 1, json!({"cache-secs": "75"}))?;
    let configured = harness.emissions("sorotte-network-options-configured")?;
    let instance_id = configured[0]["hookInstanceId"]
        .as_str()
        .expect("configured response should identify the canonical hook")
        .to_owned();
    assert!(!instance_id.is_empty());
    assert_eq!(configured[0]["currentLoadSequence"], 0);

    harness.set_path("path", "https://media.example.test/a.m3u8")?;
    harness.set_path("stream-open-filename", "https://media.example.test/a.m3u8")?;
    harness.invoke_on_load()?;
    let transition = &harness.emissions("sorotte-network-options-transition-result")?[0];
    assert_eq!(transition["hookInstanceId"], instance_id);
    assert_eq!(transition["loadSequence"], 1);

    harness.configure_as("owner-a", "attachment-a", 2, json!({"cache-secs": "90"}))?;
    let configured = harness.emissions("sorotte-network-options-configured")?;
    assert_eq!(configured[1]["hookInstanceId"], instance_id);
    assert_eq!(configured[1]["currentLoadSequence"], 1);

    harness.send(
        RELEASE_MESSAGE,
        Harness::controller_payload("owner-a", "attachment-a", 2),
    )?;
    harness.configure_as("owner-a", "attachment-b", 3, json!({"cache-secs": "120"}))?;
    let configured = harness.emissions("sorotte-network-options-configured")?;
    assert_eq!(configured[2]["hookInstanceId"], instance_id);
    assert_eq!(configured[2]["currentLoadSequence"], 1);

    harness.invoke_on_load()?;
    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions[1]["loadSequence"], 2);
    Ok(())
}

#[test]
fn live_owner_contention_is_rejected_and_takeover_follows_release_or_expiry() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as("owner-a", "attachment-a", 1, json!({"cache-secs": "75"}))?;
    harness.configure_as("owner-b", "attachment-b", 1, json!({"cache-secs": "90"}))?;
    let configured = harness.emissions("sorotte-network-options-configured")?;
    assert_eq!(configured.last().unwrap()["status"], "owner-live");

    harness.send(
        RELEASE_MESSAGE,
        Harness::controller_payload("owner-a", "attachment-a", 1),
    )?;
    harness.configure_as("owner-b", "attachment-b", 1, json!({"cache-secs": "90"}))?;
    harness.set_path("path", "https://media.example.test/b.m3u8")?;
    harness.set_path("stream-open-filename", "https://media.example.test/b.m3u8")?;
    harness.invoke_on_load()?;
    assert_eq!(
        harness.writes()?[0].1,
        "90",
        "release enables immediate takeover"
    );

    harness.advance(2.1)?;
    harness.configure_as("owner-c", "attachment-c", 1, json!({"cache-secs": "120"}))?;
    harness.set_path("path", "https://media.example.test/c.m3u8")?;
    harness.set_path("stream-open-filename", "https://media.example.test/c.m3u8")?;
    harness.invoke_on_load()?;
    assert_eq!(harness.writes()?.last().unwrap().1, "120");
    assert_eq!(
        harness
            .emissions("sorotte-network-options-configured")?
            .last()
            .unwrap()["status"],
        "configured",
        "expiry enables a different owner to take over"
    );
    Ok(())
}

#[test]
fn same_owner_attachment_contention_is_rejected_until_release() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as("owner-a", "attachment-a", 1, json!({"cache-secs": "75"}))?;
    harness.configure_as("owner-a", "attachment-b", 2, json!({"cache-secs": "90"}))?;
    let configured = harness.emissions("sorotte-network-options-configured")?;
    assert_eq!(configured.last().unwrap()["status"], "owner-live");
    assert_eq!(configured.last().unwrap()["attachmentId"], "attachment-b");
    Ok(())
}

#[test]
fn on_load_emits_local_completion_without_writes() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as("owner-a", "attachment-a", 7, json!({"cache-secs": "75"}))?;
    harness.set_path("path", "C:/media/local.mkv")?;
    harness.set_path("stream-open-filename", "C:/media/local.mkv")?;
    harness.invoke_on_load()?;
    assert!(harness.writes()?.is_empty());
    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions[0]["status"], "local");
    assert_eq!(transitions[0]["loadSequence"], 1);
    assert_eq!(transitions[0]["sourcePath"], "C:/media/local.mkv");
    assert_eq!(transitions[0]["streamOpenFilename"], "C:/media/local.mkv");
    Ok(())
}

#[test]
fn network_on_load_reports_success_and_failure_for_the_exact_sampled_path() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as(
        "owner-a",
        "attachment-a",
        7,
        json!({"cache-secs": "75", "cache-pause-wait": "5"}),
    )?;
    harness.set_path("path", "https://media.example.test/a.m3u8")?;
    harness.set_path("stream-open-filename", "https://media.example.test/a.m3u8")?;
    harness.invoke_on_load()?;

    let mut writes = harness.writes()?;
    writes.sort_unstable();
    assert_eq!(writes.len(), 2);
    assert!(
        writes
            .iter()
            .all(|(_, _, path)| path == "https://media.example.test/a.m3u8")
    );
    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions[0]["status"], "network-updated");
    assert_eq!(transitions[0]["loadSequence"], 1);
    assert_eq!(
        transitions[0]["sourcePath"],
        "https://media.example.test/a.m3u8"
    );

    harness.set_rejected_property(Some("file-local-options/cache-secs"))?;
    harness.set_path("path", "https://media.example.test/b.m3u8")?;
    harness.set_path("stream-open-filename", "https://media.example.test/b.m3u8")?;
    harness.invoke_on_load()?;
    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions[1]["status"], "failed");
    assert_eq!(transitions[1]["loadSequence"], 2);
    assert_eq!(
        transitions[1]["sourcePath"],
        "https://media.example.test/b.m3u8"
    );
    Ok(())
}

#[test]
fn rewritten_stream_target_is_classified_separately_and_same_source_loads_are_sequenced()
-> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as(
        "owner-a",
        "attachment-a",
        7,
        json!({"cache-secs": "75", "cache-pause-wait": "5"}),
    )?;
    let source = "https://service.example/watch/123";
    harness.set_path("path", source)?;
    harness.set_path("stream-open-filename", "edl://resolved-stream-a")?;
    harness.invoke_on_load()?;

    assert_eq!(
        harness.writes()?.len(),
        2,
        "the complete option map should apply"
    );
    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions[0]["status"], "network-updated");
    assert_eq!(transitions[0]["loadSequence"], 1);
    assert_eq!(transitions[0]["sourcePath"], source);
    assert_eq!(
        transitions[0]["streamOpenFilename"],
        "edl://resolved-stream-a"
    );

    harness.set_rejected_property(Some("file-local-options/cache-secs"))?;
    harness.set_path("stream-open-filename", "edl://resolved-stream-b")?;
    harness.invoke_on_load()?;
    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions[1]["status"], "failed");
    assert_eq!(transitions[1]["loadSequence"], 2);
    assert_eq!(transitions[1]["sourcePath"], source);
    assert_eq!(
        transitions[1]["streamOpenFilename"],
        "edl://resolved-stream-b"
    );
    Ok(())
}

#[test]
fn explicit_apply_reports_the_authoritative_path_used_for_its_atomic_write_set() -> mlua::Result<()>
{
    let harness = Harness::new()?;
    harness.configure_as("owner-a", "attachment-a", 7, json!({"cache-secs": "75"}))?;
    harness.set_path("path", "https://media.example.test/b.m3u8")?;
    let mut payload = Harness::controller_payload("owner-a", "attachment-a", 7);
    payload["attempt"] = json!(42);
    harness.send(APPLY_ACTIVE_MESSAGE, payload)?;

    let result = &harness.emissions("sorotte-network-options-active-result")?[0];
    assert_eq!(result["attempt"], 42);
    assert_eq!(result["loadSequence"], 0);
    assert_eq!(result["sourcePath"], "https://media.example.test/b.m3u8");
    assert_eq!(result["status"], "network-updated");
    assert!(
        harness
            .writes()?
            .iter()
            .all(|(_, _, path)| path == "https://media.example.test/b.m3u8")
    );
    Ok(())
}
