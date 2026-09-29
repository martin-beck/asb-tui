# Development/mock journey qualification

`development-journey-v1.json` is the standalone asb-tui qualification contract
for a disposable, credential-free path from development fixture enrollment to
provider/model/agent selection, mock capture, strict offline replay, and report
comparison. It is syntax and state-contract evidence only; it is not evidence
of production authentication, live-provider reachability, or secure secret
storage.

The three warning cases are deliberately non-gating in development: missing
authentication, signature validation, or key management must be shown to the
user while the mock journey continues. Network access and credentials remain
denied throughout this qualification.
