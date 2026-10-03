# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT

# Top-level TUI routing

The standalone package supplies the bounded target for the parent `asb tui`
router. The equivalent direct command is:

```console
asb-tui tui
```

It opens the interactive application. Lifecycle operations use one typed
development envelope on standard input and require an explicit development
marker:

```console
asb-tui tui status --development --json < status-request.json
asb-tui tui install --development --json < install-request.json
asb-tui tui upgrade --development --json < upgrade-request.json
asb-tui tui launch --development --json < launch-request.json
asb-tui tui remove --development --json < remove-request.json
```

For a local, credential-free development materialization, the same lifecycle
can be run without an envelope:

```console
asb-tui tui install --channel dev
asb-tui tui status --channel dev
asb-tui tui launch --channel dev
asb-tui tui upgrade --channel dev
asb-tui tui remove --channel dev
```

Lifecycle commands default to concise human-readable diagnostics. Add
`--json` for machine-readable output; the existing `--format json` spelling is
retained as a compatibility alias.

The development channel is the only currently available channel. A fresh
selection defaults to `dev`; the selected channel is projected in every
delegated lifecycle response, including install, status, launch, upgrade
(the reconfigure route), and remove. Requests for `stable`, `nightly`, or
`experimental` return a typed `<channel>_channel_unavailable` response and
never fall back to a different channel or create an installation. Existing channel
state is retained unless the user explicitly selects another available
channel.

The development install performs a shallow clone of the selected TUI source
(`ASB_TUI_DEV_REPOSITORY`, defaulting to the public asb-tui repository, and
`ASB_TUI_DEV_REF`, defaulting to `main`) into a private temporary workspace,
then runs a locked release Cargo build with private Cargo-home and target
directories. When Cargo is a rustup proxy, its trusted toolchain home must be
provided explicitly through `ASB_TUI_DEV_RUSTUP_HOME`; host `RUSTUP_HOME`,
Cargo configuration, proxy, flags, and credentials are never inherited. The
source commit and tree are read from the clone itself and
written with the executable digest to `provenance.json`; `ASB_TUI_DEV_INSTALL_ROOT`
may select an existing owner-private absolute root. Clone/build output is
streamed with a hard cap, monitored against the aggregate workspace quota, and
terminated as a process group on timeout or quota failure. Publication is
atomic and failed upgrades retain the previous pair. The response is always
classified `development_only`, and launch only reports that the materialized
executable is verified and ready. It does not claim a signed release or
production authentication.

When ASB has selected the development channel, it may provide an absolute
`ASB_TUI_CHANNEL_MANIFEST` path for the handoff. The TUI consumes that bounded
manifest during install, validates the paired ASB/TUI `main` identities and the
actual executable bytes, and stores the content digest beside the private
installation. Subsequent status and launch operations re-check the persisted
manifest and expose the paired identities in human-readable or `--json`
diagnostics. A malformed, stale, or tampered supplied manifest is a typed
development-channel error; missing provider credentials, signatures, or key
management remain visible development warnings and do not block this
prototype path.

The parent ASB command may invoke the same target as `asb tui <operation>`.
The request remains the v1 development-router envelope from
`protocol/v1/lifecycle-request.schema.json`; the selected operation and the
request operation must match. This prevents an outer command typo from
dispatching a different lifecycle action.

Development mode is explicit and warning-labelled. It uses the existing
standalone signature, artifact, ownership, atomic-activation, self-test,
supervisor, and cleanup checks; it does not require live provider
authorization or an authenticated production router. `--production` is
rejected rather than downgraded to a fixture. Public installation remains
unavailable until the release-channel gates and the external ASB router are
qualified.

## Local broker handoff

An owner-private ASB broker may expose its local control socket to the TUI:

```console
asb-tui run --socket /run/user/$UID/asb/control.sock
```

The consumer rejects relative paths, symlinks, non-sockets, shared or
group/world-readable parents, and peers owned by another uid before sending
the typed negotiation request. The broker must answer that negotiation before
any interactive request is dispatched. This is a local development transport
and does not assert production authentication or a signed release.

For the inherited `run --broker` handoff, fd 0 is the received broker control
channel. After the channel is authenticated, the TUI opens `/dev/tty` and
redirects only its interactive input there; the control stream remains a
separate owned descriptor. If no controlling terminal is available, the
command exits fail-closed instead of reading broker frames as keystrokes.

Credential-free development qualification may provide an owner-private PTY
slave through `ASB_TUI_DEVELOPMENT_TERMINAL_PATH`. The development-only seam
accepts only a bounded, non-symlink `/dev/pts/<number>` character device owned
by the current uid. Stable broker mode ignores this variable and always uses
the validated controlling-terminal path.

Exit codes are stable: `0` means the lifecycle response is successful, `2`
means command usage is invalid, and `3` means the bounded request was accepted
but the lifecycle operation was rejected or could not complete.
