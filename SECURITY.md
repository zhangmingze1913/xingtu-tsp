# Security Policy

## Supported scope

This repository is a TSP reference/POC baseline, not a certified compliance product. Security fixes are accepted for the current `main` branch.

## Reporting a vulnerability

Do **not** open a public issue for:

- authentication/authorization bypass
- secret or PII leakage
- remote command injection
- TLS/mTLS bypass
- export encryption/key handling issues
- unsafe default credentials

Report privately to the repository maintainer with:

```text
affected version/commit
reproduction steps
impact
suggested fix (optional)
```

Do not include real customer data, production keys or terminal credentials.

## Default credentials

The demo seed contains documented development credentials:

```text
admin / admin123
operator / operator123
viewer / viewer123
```

They are for local demonstration only. Production startup rejects default admin credentials and requires `TSP_ENV=production` safeguards.

## Secrets

Never commit:

```text
.env
*.pem (except tests/fixtures/tls, which is a test-only self-signed fixture)
license.dat
export encryption keys
JWT private keys
RustFS/Vault/TDengine credentials
```

Use Kubernetes Secrets, Vault or an external KMS.

## Test TLS fixture

`tests/fixtures/tls/` contains a self-signed certificate and private key used only by local TLS tests. It has no production trust and must not be reused.

## Disclosure

Please allow reasonable time for a fix before public disclosure. This project does not currently operate a bug bounty program.
