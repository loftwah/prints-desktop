# Read-only Oyster connection

The desktop preview opens its bundled workspace offline. The optional Rust
diagnostic exercises the direct Prints read contract for developers who have
an authorised Oyster installation.

## Requirements

Provide the Oyster HTTPS origin, the enabled Prints installation ID, and a
revocable named App key with `apps.read` for that specific installation. The
installation must already have the appropriate library, Computer and media
permissions. Confirm the intended Computer in Oyster before connecting. The
selected Computer and library can change on the host, so integrations must
recheck that identity before caching or handing off owner assets.

The existing host returned HTTP 401 on the typed library method without a
scoped App key. An authenticated test against that host remains open.

- [Issue, specification and live evidence](https://github.com/loftwah/app-composer/issues/1292)
- [Fungi guide to calling an App with a scoped key](https://docs.fungi.computer/guides/call-an-app-from-your-own-server/)
- [Self-hosting Oyster](https://docs.fungi.computer/guides/run-your-own-oyster/)
- [Prints on Workers AI](https://docs.fungi.computer/apps/prints)

## Diagnostic

Run this from the repository root with an installation you are authorised to
read. The terminal prompts for the key without echoing it. The process holds
it in memory and prints only aggregate counts.

```sh
cargo run --locked --features oyster-reader --example oyster-reader -- \
  'https://your-oyster.example' 'prints'
```

Someone who already knows an authorised print ID can append
`--export <print-id> <new-file>`. The diagnostic creates a new local file from
the bounded original byte stream. The path must be one the caller selects.
It refuses an existing destination.

## Protocol and evidence

| Method | Purpose | Installation permission |
| --- | --- | --- |
| `listModels` | Discover the admitted model catalogue | `media.generation` |
| `listOutputs` | Browse indexed output records and a page cursor | `computer.list` |
| `openOutput` | Read base64 chunks of an indexed original | `computer.readFile` |

The client sends fixed JSON requests to
`POST /apps/<installationId>/methods/<method>` with the key in the bearer
header. TLS is required outside loopback. Redirects, proxy discovery and
broad owner credentials are rejected. The diagnostic performs read calls
only, ignores untrusted error text, and limits response size, chunk count
and download time. A chunk is at most 128 KiB. The upstream empty content
slice ends the stream. The desktop preview does not install this diagnostic.

The typed route and permissions come from
[Oyster 0.58.9 at 8295ec6](https://github.com/fungi-computer/oyster/blob/8295ec63a32d09734d887b01d7675788a0639fbd/src/host.ts#L2722-L2739)
and [Prints 0.21.17 at 8482912](https://github.com/fungi-computer/prints/blob/8482912387552cc602d54b0d41ec4271c9cb7f43/backend/methods.ts).
The first live test needs a known installation and sample asset. Confirm model
and library access, compare the retrieved bytes against the same owner
export, then verify denial after key revocation. Test a different installation
and a moved Computer selection before persisting a connection in the app.

Read-only transport tests use an isolated loopback server with fictional
responses. Run them with:

```sh
cargo test --locked --features oyster-reader --lib
```

A hosted generation job needs separate cost and dispatch authority. Confirm
the installed model, quote, request identity and same-key recovery before
dispatching. Cloudflare infrastructure, Workers AI and any external inference
have separate usage and billing.
