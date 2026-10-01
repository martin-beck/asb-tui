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
asb-tui tui status --development --format json < status-request.json
asb-tui tui install --development --format json < install-request.json
asb-tui tui upgrade --development --format json < upgrade-request.json
asb-tui tui launch --development --format json < launch-request.json
asb-tui tui remove --development --format json < remove-request.json
```

For a local, credential-free development materialization, the same lifecycle
can be run without an envelope:

```console
asb-tui tui install --channel dev --format json
asb-tui tui status --channel dev --format json
asb-tui tui launch --channel dev --format json
asb-tui tui upgrade --channel dev --format json
asb-tui tui remove --channel dev --format json
```

The development channel is the only currently available channel. A fresh
selection defaults to `dev`; the selected channel is projected in every
delegated lifecycle response, including install, status, launch, upgrade
(the reconfigure route), and remove. Requests for `stable`, `nightly`, or
`experimental` return a typed `<channel>_channel_unavailable` response and
never fall back to a different channel or create an installation. Existing channel
state is retained unless the user explicitly selects another available
channel.

This copies the current executable into an owner-private temporary root and
writes `provenance.json`; `ASB_TUI_DEV_INSTALL_ROOT` may select an existing
owner-private absolute root. The response is always classified
`development_only`, and launch only reports that the materialized executable
is verified and ready. It does not claim a signed release or production
authentication.

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

Exit codes are stable: `0` means the lifecycle response is successful, `2`
means command usage is invalid, and `3` means the bounded request was accepted
but the lifecycle operation was rejected or could not complete.
