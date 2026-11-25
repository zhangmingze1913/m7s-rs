# Security Policy

Please do not open a public issue for security vulnerabilities.

Use GitHub's **Security → Advisories → Report a vulnerability** for this
repository. Include:

- affected commit/version;
- protocol or component;
- reproduction steps or a minimal PoC;
- impact assessment (auth bypass, RCE, SSRF, DoS, etc.).

We will acknowledge reports and coordinate a fix. Test fixtures under
`tests/fixtures/` contain **test-only** certificates and private keys; they must
never be reused in production.

## Supported versions

Only the latest `main` and the latest tagged release receive security fixes
while the project is pre-1.0.
