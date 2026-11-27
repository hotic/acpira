//! Plans, slash commands, config options, modes and usage snapshots

use super::*;

#[test]
fn a_later_plan_replaces_the_whole_list_within_the_turn() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "plan", "entries": [{ "content": "a", "priority": "high", "status": "in_progress" }, { "content": "b", "priority": "low", "status": "pending" }] }));
  apply(&mut s, json!({ "sessionUpdate": "plan", "entries": [{ "content": "a", "priority": "high", "status": "completed" }, { "content": "b", "priority": "low", "status": "in_progress" }] }));
  let t = v(&s.turns[0]);
  assert_eq!(t["blocks"].as_array().unwrap().len(), 1);
  expect_eq(&t["blocks"][0], json!({ "type": "plan", "changed": true,
    "entries": [{ "title": "a", "status": "completed", "priority": "high" }, { "title": "b", "status": "in_progress", "priority": "low" }] }));
}

#[test]
fn available_commands_keep_the_input_hint_drop_meta_and_are_replaced_by_later_lists() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "available_commands_update", "availableCommands": [
    { "name": "compact", "description": "Compact", "_meta": { "x": 1 } },
    { "name": "review", "description": "Review", "input": { "hint": "files to review", "_meta": { "y": 2 } } },
    { "name": "plain", "description": "No hint", "input": null },
  ] }));
  expect_eq(&s.commands, json!([
    { "name": "compact", "description": "Compact" },
    { "name": "review", "description": "Review", "input": { "hint": "files to review" } },
    { "name": "plain", "description": "No hint" },
  ]));
  apply(&mut s, json!({ "sessionUpdate": "available_commands_update", "availableCommands": [{ "name": "review", "description": "Review" }] }));
  assert_eq!(s.commands.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["review"]);
  apply(&mut s, json!({ "sessionUpdate": "available_commands_update", "availableCommands": [] }));
  assert!(s.commands.is_empty());
}

#[test]
fn config_options_become_controls_sorted_by_category_with_mode_promoted() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "config_option_update", "configOptions": [
    { "id": "verbose", "name": "Verbose", "type": "boolean", "currentValue": true },
    { "id": "custom", "name": "Style", "type": "select", "currentValue": "x", "options": [{ "value": "x", "name": "X" }] },
    { "id": "effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": "hi", "options": [{ "value": "lo", "name": "Lo" }, { "value": "hi", "name": "Hi" }] },
    { "id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "plan", "options": [{ "value": "agent", "name": "Agent" }, { "value": "plan", "name": "Plan" }] },
    { "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "b",
      "options": [{ "group": "g1", "name": "Group 1", "options": [{ "value": "a", "name": "A" }] }, { "group": "g2", "name": "Group 2", "options": [{ "value": "b", "name": "B" }] }] },
  ] }));
  assert_eq!(s.controls.options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["model", "effort", "verbose", "custom"]);
  // A boolean option is a string-valued control everywhere but the wire (synthetic Off/On pair, value stringified)
  expect_eq(&s.controls.options[2], json!({ "id": "verbose", "name": "Verbose", "type": "boolean", "value": "true", "options": [{ "id": "false", "name": "Off" }, { "id": "true", "name": "On" }] }));
  expect_eq(&s.controls.options[0].options, json!([
    { "id": "a", "name": "A", "description": "Group 1", "group": { "id": "g1", "name": "Group 1" } },
    { "id": "b", "name": "B", "description": "Group 2", "group": { "id": "g2", "name": "Group 2" } },
  ]));
  assert_eq!(s.controls.options[0].value.as_deref(), Some("b"));
  assert_eq!(s.controls.modes.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["agent", "plan"]);
  assert_eq!(s.controls.mode_id.as_deref(), Some("plan"));
  assert_eq!(s.controls.mode_config_id.as_deref(), Some("mode"));
}

#[test]
fn a_mode_config_option_duplicating_modes_stays_out_of_the_controls() {
  let mut s = state();
  s.controls = serde_json::from_value(json!({ "modes": [{ "id": "default", "name": "Default" }, { "id": "yolo", "name": "YOLO" }], "modeId": "default", "options": [] })).unwrap();
  apply(&mut s, json!({ "sessionUpdate": "config_option_update", "configOptions": [
    { "id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "default", "options": [{ "value": "default", "name": "Default" }, { "value": "yolo", "name": "YOLO" }] },
    { "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "k3", "options": [{ "value": "k3", "name": "K3" }] },
  ] }));
  assert_eq!(s.controls.options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["model"]);
  assert_eq!(s.controls.modes.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["default", "yolo"]);
  assert_eq!(s.controls.mode_id.as_deref(), Some("default"));
  assert!(s.controls.mode_config_id.is_none());
}

#[test]
fn current_mode_updates_replace_the_advertised_list_without_resetting_model_controls() {
  let mut s = state();
  init_controls(&mut s.controls, Some(&json!({ "currentModeId": "agent", "availableModes": [
    { "id": "agent", "name": "Agent" }, { "id": "plan", "name": "Plan" }
  ] })), Some(&json!([
    { "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "m1", "options": [{ "value": "m1", "name": "Model 1" }] }
  ])));
  let options = s.controls.options.clone();
  assert!(apply(&mut s, json!({ "sessionUpdate": "current_mode_update", "currentModeId": "agent", "_meta": { "acpira/availableModes": [
    { "id": "agent", "name": "Agent" }, { "id": "plan", "name": "Plan" }, { "id": "raw", "name": "Raw", "description": "No tools" }
  ] } })));
  assert_eq!(s.controls.mode_id.as_deref(), Some("agent"));
  assert_eq!(s.controls.modes.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["agent", "plan", "raw"]);
  assert_eq!(s.controls.modes[2].description.as_deref(), Some("No tools"));
  assert_eq!(s.controls.options, options);
  // Ordinary ACP current-mode updates still only change the selection
  apply(&mut s, json!({ "sessionUpdate": "current_mode_update", "currentModeId": "raw" }));
  assert_eq!(s.controls.mode_id.as_deref(), Some("raw"));
  assert_eq!(s.controls.modes.len(), 3);
  apply(&mut s, json!({ "sessionUpdate": "current_mode_update", "currentModeId": "agent", "_meta": { "acpira/availableModes": [
    { "id": "agent", "name": "Agent" }, { "id": "plan", "name": "Plan" }
  ] } }));
  assert_eq!(s.controls.modes.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["agent", "plan"]);
  assert_eq!(s.controls.options, options);
}

#[test]
fn usage_update_stamps_the_context_snapshot_on_the_trailing_agent_turn_only() {
  let mut s = state();
  s.turns.push(turn(json!({ "role": "user", "text": "hi" })));
  s.turns.push(turn(json!({ "role": "agent", "blocks": [] })));
  apply(&mut s, json!({ "sessionUpdate": "usage_update", "used": 5000, "size": 100_000 }));
  expect_eq(s.usage, json!({ "used": 5000, "size": 100_000 }));
  expect_match(&s.turns[1], json!({ "usage": { "context": { "used": 5000, "size": 100_000 } } }));
  // Per-call updates keep overwriting: the last one before end_turn is the end-of-turn snapshot
  apply(&mut s, json!({ "sessionUpdate": "usage_update", "used": 7000, "size": 100_000 }));
  expect_match(&s.turns[1], json!({ "usage": { "context": { "used": 7000, "size": 100_000 } } }));

  // A trailing user turn is left alone: the snapshot never reaches back across it
  let mut t = state();
  t.turns.push(turn(json!({ "role": "agent", "blocks": [] })));
  t.turns.push(turn(json!({ "role": "user", "text": "next" })));
  apply(&mut t, json!({ "sessionUpdate": "usage_update", "used": 1, "size": 2 }));
  expect_absent(&t.turns[0], "usage");
  expect_absent(&t.turns[1], "usage");
}

#[test]
fn boolean_controls_send_a_real_boolean_on_the_wire() {
  let bool_: ConfigControl = serde_json::from_value(json!({ "id": "fast", "name": "Fast", "type": "boolean", "value": "false", "options": [{ "id": "false", "name": "Off" }, { "id": "true", "name": "On" }] })).unwrap();
  expect_eq(config_option_set_value(Some(&bool_), "true"), json!({ "type": "boolean", "value": true }));
  expect_eq(config_option_set_value(Some(&bool_), "false"), json!({ "type": "boolean", "value": false }));
  let sel: ConfigControl = serde_json::from_value(json!({ "id": "model", "name": "Model", "value": "a", "options": [{ "id": "a", "name": "A" }] })).unwrap();
  expect_eq(config_option_set_value(Some(&sel), "b"), json!({ "value": "b" }));
  expect_eq(config_option_set_value(None, "x"), json!({ "value": "x" }));
}

#[test]
fn mode_meta_kind_lands_on_the_session_option() {
  let mut s = state();
  init_controls(&mut s.controls, Some(&json!({ "currentModeId": "default", "availableModes": [
    { "id": "default", "name": "Default" },
    { "id": "full", "name": "Full access", "_meta": { "kind": "full_access" } },
  ] })), None);
  assert_eq!(v(&s.controls.modes).as_array().unwrap().iter().map(|m| m["kind"].as_str()).collect::<Vec<_>>(), [None, Some("full_access")]);
}

#[test]
fn a_mode_config_select_keeps_meta_kind_on_its_options() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "config_option_update", "configOptions": [
    { "id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "default", "options": [
      { "value": "default", "name": "Default" },
      { "value": "full", "name": "Full access", "_meta": { "kind": "full_access" } },
    ] },
  ] }));
  assert_eq!(v(&s.controls.modes).as_array().unwrap().iter().map(|m| m["kind"].as_str()).collect::<Vec<_>>(), [None, Some("full_access")]);
}
