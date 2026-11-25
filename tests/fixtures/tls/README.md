# Test-only TLS fixture

`ca.pem`, `server.pem` and `server-key.pem` are a self-signed local test
certificate chain used by `tests/standards.rs` / JT808 TLS tests.

They are **not** production credentials and must never be trusted or reused.
Secret scanners may flag `server-key.pem`; this is expected for a test fixture.
