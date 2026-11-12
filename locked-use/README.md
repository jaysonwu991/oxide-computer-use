# Oxide Computer Use locked use

This is a separate security-component design for macOS locked use. It exists so
the normal `oxide-computer-use` stdio server never contains unlock privileges or
password handling.

The crate is intentionally pure:

- it evaluates a short-lived trusted-turn grant;
- it requires a trusted signed bundle, matching identity, covered displays and
  no local input;
- every missing or stale condition returns a denial;
- the only positive result is permission for a separate privileged host to begin
  its own narrowly scoped authorization flow.

It is not an Authorization Plug-in, does not call any unlock API and must not be
installed until the controls in [THREAT_MODEL.md](THREAT_MODEL.md) have a signed
backend, installer, audit and security review.
