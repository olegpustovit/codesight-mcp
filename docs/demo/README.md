# Stdio MCP capture

Captured on 23 October 2026 from base revision `426c390961830ac0f2ba0a613010f499cdc6f0a6` with the documentation and fixture additions in this commit.

The source binary built with Rust 1.96.0 on Linux ARM64. The first build ran out of temporary disk space. After removing completed-task build outputs, the same source built with CARGO_PROFILE_DEV_DEBUG=0 and CARGO_INCREMENTAL=0. No product source was changed to make the build pass.

Commands from the repository root:

```sh
cargo build --locked --bin codesight-mcp
node docs/demo/run.cjs
```

The harness starts the actual stdio server in the included synthetic workspace with a temporary index. It performs MCP initialization, lists the six tools, outlines math.js, reads lines 1-3 and checks that an impact request for a missing file fails. results.json contains the actual envelopes; output.txt extracts their fields for readability. The PNG is a browser-rendered transcript of that summary. Confidence labels in it come from the server and do not establish completeness of other analyses.

No live agent was configured, no private repository was indexed and no full premium or cross-platform gate was run. The broader language and performance claims in historical reference examples remain outside this fixture's verification scope.
