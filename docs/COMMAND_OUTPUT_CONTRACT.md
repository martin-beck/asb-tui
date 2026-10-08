<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->

# Guided command and output contract

The command entry points are route selectors for the guided TUI.  They do not
accept provider credentials, paths, endpoints, or opaque run identifiers on
the command line.  With no output option they print a short human-readable
next step; `--json` is the preferred machine-readable spelling and
`--format json` is retained as a compatibility alias.

| Selector (aliases) | Guided destination | Default output | JSON command | Unavailable exit |
| --- | --- | --- | --- | --- |
| `setup` (`wizard`) | provider and defaults | human | `setup --json` | 3 |
| `benchmark` | workload and run | human | `benchmark --format json` | 3 |
| `record` (`recording`) | recording | human | `record --json` | 3 |
| `replay` | offline replay | human | `replay --json` | 3 |
| `compare` (`comparison`) | result comparison | human | `compare --json` | 3 |
| `status` | connection and run status | human | `status --json` | 3 |
| `doctor` | readiness checks | human | `doctor --json` | 3 |
| `upgrade` | verified upgrade | human | `upgrade --json` | 3 |
| `remove` (`uninstall`) | safe removal | human | `remove --json` | 3 |

JSON output is one object with `schema_version`, `command`, `status`,
`message`, `next_action`, and (when applicable)
`development_warning`.  The warning explicitly marks missing development
credentials/signatures/key management as non-blocking.  No output field
contains a secret, endpoint, local path, environment value, or provider
payload.  A channel-unavailable result belongs to the selected lifecycle
route; it is not reported as a generic benchmark protocol failure.

For a source checkout or the default `dev` channel, this warning is
informational: an absent provider API key, release signature, or local key
manager must not prevent opening the guided setup screen or continuing to
configure a development run.  The TUI may show the missing capability and
offer the next setup action, but it must not turn it into an authentication
prompt that blocks the prototype.  This development boundary does not weaken
published-release verification: production bundle installation and upgrade
still fail closed when an authenticated manifest or its trust anchor is
missing or invalid.

The existing `doctor --format json`, `compatibility --format json`, and
lifecycle JSON protocols remain unchanged for callers that already consume
those compatibility interfaces.

The broker's v1.15 dynamic-provider-catalog result is a separate authenticated JSON control
projection, not a guided command result. Human setup status maps its closed modes and diagnostics to
stable labels. The JSON wire retains only the typed `unavailable`, `http_status`, `too_large`, and
`malformed` diagnostics; HTTP 401/403 is presented as `authentication_required` and HTTP 429 as
`quota_unavailable`. Older negotiated versions report `control_version_unsupported`. Neither form
contains provider payloads, credentials, endpoints, environment values, or a fallback claim.
