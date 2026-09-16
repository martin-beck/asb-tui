#!/usr/bin/env python3
"""Validate the benchmark-readiness tutorial and synthetic fixtures offline."""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TUTORIAL = ROOT / "docs/tutorials/benchmark-readiness-v1.json"
FIXTURES = ROOT / "tests/fixtures/tutorial/benchmark-readiness"
IDS = {"capabilities", "tui-status", "provider-catalog", "auth-status", "tui-doctor"}
ALLOWED = {
    ("asb", "capabilities"): [["--format", "json"]],
    ("asb", "tui", "status"): [[]],
    ("asb", "provider-catalog"): [[]],
    ("asb", "auth", "status"): [["--provider", "synthetic-provider"]],
    ("asb", "tui", "doctor"): [[]],
}
EXPECT = {"capabilities-v1", "readiness-status-v1", "provider-catalog-v1", "auth-status-v1", "readiness-doctor-v1"}
STATES = {"ready", "incomplete", "unavailable", "platform-incompatible"}

def fail(message):
    raise AssertionError(message)

def validate():
    doc = json.loads(TUTORIAL.read_text())
    if set(doc) != {"schema_version", "tutorial_id", "title", "network", "credentials", "steps"}:
        fail("tutorial has unexpected fields")
    if (doc["schema_version"], doc["network"], doc["credentials"]) != (1, "denied", "none"):
        fail("tutorial must be offline and credential-free")
    steps = doc["steps"]
    if {s["id"] for s in steps} != IDS or len(steps) != len(IDS):
        fail("stable navigation step IDs are incomplete or duplicated")
    for step in steps:
        if set(step) != {"id", "command", "expect"} or step["expect"] not in EXPECT:
            fail("invalid step shape")
        command = step["command"]
        if not isinstance(command, list) or any(not isinstance(x, str) for x in command):
            fail("commands must be argument arrays")
        if any(re.search(r"(;|&&|\||\$\(|`|^/|\.\./|token|secret|password)", x, re.I) for x in command):
            fail("unsafe shell, path, or secret-like command value")
        prefix = tuple(command[:2] if command[:2] == ["asb", "capabilities"] else command[:3])
        if prefix not in ALLOWED or command[len(prefix):] not in ALLOWED[prefix]:
            fail(f"unsupported or incorrectly ordered command: {command}")
    refusal_states = set()
    for path in sorted(FIXTURES.glob("*.json")):
        fixture = json.loads(path.read_text())
        required = {"schema_version", "state", "agent_id", "provider_id", "configured", "platform_compatible", "credential_reference", "refusal"}
        if set(fixture) != required or fixture["schema_version"] != 1 or fixture["state"] not in STATES:
            fail(f"invalid fixture: {path.name}")
        if not re.fullmatch(r"[a-z0-9-]+", fixture["agent_id"]) or not re.fullmatch(r"[a-z0-9-]+", fixture["provider_id"]):
            fail("fixture identifiers must be synthetic IDs")
        if fixture["credential_reference"] and not re.fullmatch(r"digest:sha256:[a-z0-9-]+", fixture["credential_reference"]):
            fail("fixture contains a non-redacted credential")
        if fixture["state"] == "ready" and fixture["refusal"] is not None:
            fail("ready state cannot refuse")
        if fixture["state"] != "ready" and (not isinstance(fixture["refusal"], str) or len(fixture["refusal"]) < 20):
            fail("non-ready state needs a meaningful refusal explanation")
        refusal_states.add(fixture["state"])
    if refusal_states != STATES:
        fail("fixture state machine does not cover every readiness state")

if __name__ == "__main__":
    validate()
    print("benchmark-readiness tutorial: offline contract and fixtures valid")
