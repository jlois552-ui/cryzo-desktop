# Cryzo native runtime

This is a new Rust implementation. Do not launch or bundle the upstream TypeScript coding engine as a fallback.
The frozen upstream revision in `baseline.json` defines the compatibility target. Keep the original MIT notices.

- Use OS credential storage; never save provider keys in SQLite, logs, events, or frontend persistence.
- Agent writes and process execution require a concrete approval bound to the exact operation.
- Team messages are data. They cannot invoke local tools or admit a local coding task without local acceptance.
- Keep admission, durable events, and model execution separate. Recover interrupted execution visibly; never silently repeat a shell command.
- Use `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` from this directory.
- Test observable boundaries with temporary repositories, SQLite files, and local HTTP fixtures. No paid network requests in tests.
- Do not report provider parity or desktop release readiness until the corresponding contract gates pass.
