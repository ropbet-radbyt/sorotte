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
local original_tostring = tostring
function tostring(value)
    if type(value) == 'table' and __fixed_table_tostring ~= nil then
        return __fixed_table_tostring
    end
    return original_tostring(value)
end
os.time = function() return __wall_clock_time end
__harness = {
    messages = {}, hooks = {}, events = {}, timers = {}, properties = {}, writes = {}, emissions = {},
    reject = nil, time = __start_time,
}
mp = { keep_running = true }
function mp.get_script_name() return __script_name end
function mp.register_script_message(name, callback) __harness.messages[name] = callback end
function mp.register_event(name, callback) __harness.events[name] = callback end
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
    local option_name = name:match('^file%-local%-options/(.+)$')
    if option_name ~= nil then __harness.properties[option_name] = value end
    return true
end
function mp.commandv(...)
    local args = {...}
    if args[1] == 'script-message' then
        table.insert(__harness.emissions, { name = args[2], payload = args[3] })
    end
end
package.preload['mp.utils'] = function()
    return { parse_json = __parse_json, format_json = __format_json, getpid = function() return __pid end }
end
"#;

#[derive(Clone, Copy)]
struct HookInstanceIdentityInputs {
    pid: u32,
    wall_clock_time: i64,
    start_time: f64,
    fixed_table_tostring: &'static str,
}

impl Default for HookInstanceIdentityInputs {
    fn default() -> Self {
        Self {
            pid: 4_242,
            wall_clock_time: 1_700_000_000,
            start_time: 10.0,
            fixed_table_tostring: "table: fixed-hook-anchor",
        }
    }
}

struct Harness {
    lua: Lua,
}

impl Harness {
    fn new() -> mlua::Result<Self> {
        Self::new_with_identity_inputs(HookInstanceIdentityInputs::default())
    }

    fn new_with_identity_inputs(inputs: HookInstanceIdentityInputs) -> mlua::Result<Self> {
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
        lua.globals().set("__pid", inputs.pid)?;
        lua.globals()
            .set("__wall_clock_time", inputs.wall_clock_time)?;
        lua.globals().set("__start_time", inputs.start_time)?;
        lua.globals()
            .set("__fixed_table_tostring", inputs.fixed_table_tostring)?;
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

    fn invoke_file_loaded(&self) -> mlua::Result<()> {
        let events: Table = self.table()?.get("events")?;
        let callback: Function = events.get("file-loaded")?;
        callback.call(())
    }

    fn invoke_end_file(&self) -> mlua::Result<()> {
        let events: Table = self.table()?.get("events")?;
        let callback: Function = events.get("end-file")?;
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
    harness.invoke_file_loaded()?;
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
    harness.invoke_file_loaded()?;
    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions[1]["loadSequence"], 2);
    Ok(())
}

#[test]
fn reloaded_hook_instances_differ_when_pid_wall_clock_and_table_identity_collide()
-> mlua::Result<()> {
    let first = Harness::new_with_identity_inputs(HookInstanceIdentityInputs {
        start_time: 10.25,
        ..HookInstanceIdentityInputs::default()
    })?;
    let second = Harness::new_with_identity_inputs(HookInstanceIdentityInputs {
        start_time: 10.5,
        ..HookInstanceIdentityInputs::default()
    })?;

    first.configure_as("owner-a", "attachment-a", 1, json!({"cache-secs": "75"}))?;
    first.configure_as("owner-a", "attachment-a", 2, json!({"cache-secs": "90"}))?;
    second.configure_as("owner-b", "attachment-b", 1, json!({"cache-secs": "75"}))?;

    let first_configured = first.emissions("sorotte-network-options-configured")?;
    let second_configured = second.emissions("sorotte-network-options-configured")?;
    let first_instance = &first_configured[0]["hookInstanceId"];
    let second_instance = &second_configured[0]["hookInstanceId"];

    assert_eq!(
        &first_configured[1]["hookInstanceId"], first_instance,
        "one Lua runtime must retain a stable hook instance id"
    );
    assert_ne!(
        first_instance, second_instance,
        "separately loaded hooks must not collide when PID, wall clock, and table tostring do"
    );
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
    harness.invoke_file_loaded()?;

    let writes = harness.writes()?;
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].0, "file-local-options/cache-pause-wait");
    assert_eq!(writes[1].0, "file-local-options/cache-secs");
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
    harness.invoke_file_loaded()?;
    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions[1]["status"], "failed");
    assert_eq!(transitions[1]["applicationState"], "partially-applied");
    assert_eq!(transitions[1]["loadSequence"], 2);
    assert_eq!(
        transitions[1]["sourcePath"],
        "https://media.example.test/b.m3u8"
    );
    assert_eq!(
        transitions[1]["optionResults"],
        json!([
            {"name": "cache-pause-wait", "status": "applied"},
            {"name": "cache-secs", "status": "rejected"},
        ])
    );
    assert_eq!(
        harness.writes()?[3].0,
        "file-local-options/cache-secs",
        "the rejected option should still be recorded in deterministic order"
    );
    Ok(())
}

#[test]
fn network_on_load_prefers_http2_when_curl_protocol_selection_is_automatic() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as("owner-a", "attachment-a", 7, json!({"cache-secs": "75"}))?;
    harness.set_path("options/curl-http-version", "auto")?;
    harness.set_path("path", "https://media.example.test/video")?;
    harness.set_path("stream-open-filename", "https://media.example.test/video")?;

    harness.invoke_on_load()?;

    let writes = harness.writes()?;
    assert_eq!(
        writes[0],
        (
            "file-local-options/curl-http-version".to_owned(),
            "2tls".to_owned(),
            "https://media.example.test/video".to_owned(),
        )
    );
    assert_eq!(writes[1].0, "file-local-options/cache-secs");

    let explicit = Harness::new()?;
    explicit.configure_as("owner-a", "attachment-a", 7, json!({"cache-secs": "75"}))?;
    explicit.set_path("options/curl-http-version", "3only")?;
    explicit.set_path("path", "https://media.example.test/video")?;
    explicit.set_path("stream-open-filename", "https://media.example.test/video")?;
    explicit.invoke_on_load()?;
    assert_eq!(explicit.writes()?.len(), 1);
    assert_eq!(explicit.writes()?[0].0, "file-local-options/cache-secs");
    Ok(())
}

#[test]
fn file_loaded_readback_reports_effective_values_after_all_writes() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as(
        "owner-a",
        "attachment-a",
        9,
        json!({
            "cache": "auto",
            "cache-secs": "30",
            "demuxer-max-bytes": "150MiB",
            "cache-on-disk": "no",
            "ytdl-format": "best",
        }),
    )?;
    harness.set_path("path", "https://media.example.test/readback.m3u8")?;
    harness.set_path(
        "stream-open-filename",
        "https://media.example.test/readback.m3u8",
    )?;
    harness.invoke_on_load()?;
    assert!(
        harness
            .emissions("sorotte-network-options-transition-result")?
            .is_empty(),
        "a network write acknowledgement is not authoritative until file-loaded readback"
    );
    harness.set_path("cache-secs", "45")?;
    harness.invoke_file_loaded()?;

    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0]["verification"], "complete");
    assert_eq!(transitions[0]["configurationGeneration"], 9);
    assert_eq!(transitions[0]["loadSequence"], 1);
    assert_eq!(transitions[0]["effectiveOptions"]["cache"], "auto");
    assert_eq!(transitions[0]["effectiveOptions"]["cache-secs"], "45");
    assert_eq!(
        transitions[0]["effectiveOptions"]["demuxer-max-bytes"],
        "150MiB"
    );
    assert_eq!(transitions[0]["effectiveOptions"]["cache-on-disk"], "no");
    assert!(
        transitions[0]["effectiveOptions"]
            .get("ytdl-format")
            .is_none()
    );
    Ok(())
}

#[test]
fn failed_load_terminally_reports_and_clears_pending_verification() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as(
        "owner-a",
        "attachment-a",
        9,
        json!({"cache-pause-wait": "5", "cache-secs": "30"}),
    )?;
    harness.set_path(
        "path",
        "https://media.example.test/fails-before-file-loaded",
    )?;
    harness.set_path(
        "stream-open-filename",
        "https://media.example.test/fails-before-file-loaded",
    )?;
    harness.invoke_on_load()?;
    assert!(
        harness
            .emissions("sorotte-network-options-transition-result")?
            .is_empty()
    );

    harness.invoke_end_file()?;
    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0]["status"], "failed");
    assert_eq!(transitions[0]["applicationState"], "failed");
    assert_eq!(transitions[0]["verification"], "incomplete");
    assert_eq!(
        transitions[0]["optionResults"],
        json!([
            {"name": "cache-pause-wait", "status": "applied"},
            {"name": "cache-secs", "status": "applied"},
        ])
    );

    harness.invoke_file_loaded()?;
    assert_eq!(
        harness
            .emissions("sorotte-network-options-transition-result")?
            .len(),
        1,
        "a later event must not resurrect the failed load or its retained target"
    );
    Ok(())
}

#[test]
fn rejected_option_does_not_stop_later_deterministically_ordered_writes() -> mlua::Result<()> {
    let harness = Harness::new()?;
    harness.configure_as(
        "owner-a",
        "attachment-a",
        10,
        json!({
            "cache": "auto",
            "cache-secs": "30",
            "demuxer-max-bytes": "150MiB",
        }),
    )?;
    harness.set_rejected_property(Some("file-local-options/cache-secs"))?;
    harness.set_path("path", "https://media.example.test/partial.m3u8")?;
    harness.set_path(
        "stream-open-filename",
        "https://media.example.test/partial.m3u8",
    )?;
    harness.invoke_on_load()?;
    harness.invoke_file_loaded()?;

    let writes = harness.writes()?;
    assert_eq!(
        writes
            .iter()
            .map(|write| write.0.as_str())
            .collect::<Vec<_>>(),
        vec![
            "file-local-options/cache",
            "file-local-options/cache-secs",
            "file-local-options/demuxer-max-bytes",
        ]
    );
    let result = &harness.emissions("sorotte-network-options-transition-result")?[0];
    assert_eq!(result["status"], "failed");
    assert_eq!(result["applicationState"], "partially-applied");
    assert_eq!(
        result["optionResults"],
        json!([
            {"name": "cache", "status": "applied"},
            {"name": "cache-secs", "status": "rejected"},
            {"name": "demuxer-max-bytes", "status": "applied"},
        ])
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
    harness.invoke_file_loaded()?;

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
    harness.invoke_file_loaded()?;
    let transitions = harness.emissions("sorotte-network-options-transition-result")?;
    assert_eq!(transitions[1]["status"], "failed");
    assert_eq!(transitions[1]["applicationState"], "partially-applied");
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
