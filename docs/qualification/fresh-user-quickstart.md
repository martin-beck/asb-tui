# AR-1643 executable fresh-user quickstart

From a clean checkout, build and run the complete development journey with:

```text
python3 tools/run-fresh-user-quickstart.py \
  --asb-checkout /path/to/agent-systems-benchmark \
  --receipt /tmp/ar-1643-quickstart.json --json
```

The runner builds missing binaries with `cargo build --locked`, then delegates
to the existing acceptance seam. The journey explicitly exercises
`asb tui install` followed by `asb tui` status/launch, wizard/provider setup,
selected benchmark execution, capture/seal, strict offline replay, comparison,
and analysis. It uses development fixtures when credentials are absent,
scrubs secret-like environment variables, and denies network for benchmark and
replay operations. The JSON receipt binds both source heads and contains no
workspace paths or credentials.
