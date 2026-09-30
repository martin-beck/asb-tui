<!-- Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved. -->
<!-- SPDX-License-Identifier: MIT -->
# End-to-end development journey qualification

`end-to-end-journey-v1.json` is the AR-1338 acceptance contract for a
disposable, credential-free development fixture. It covers the complete
visible journey from `asb tui install` and `asb tui` through wizard setup,
catalog selection, configuration materialization and preflight, benchmark
launch, live statistics, final results, history, and comparison.

The executable evidence is `tests/end_to_end_qualification.rs`. It records a
bounded transcript of user-visible actions and exercises the same renderer-
neutral seams used by the application. The fixture denies network access,
contains no credentials, and labels every result `development/mock`; it is not
evidence of live-provider reachability, production authentication, or release
readiness.

The test also verifies that version, catalog, bundle, and protocol mismatches
stop before execution and expose exactly one actionable recovery choice.
Generated configuration is private, canonical, digest-bound JSON and is
reloaded from a disposable store before the launch handoff is accepted.
