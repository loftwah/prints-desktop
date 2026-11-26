# Read-only Oyster connection

The Loftwah Creative Archive and Afterlight work offline. The **Connect Prints** screen accepts a named App
key through a secure input. Choose **Remember key on this device** to save it
through Keychain, Windows Credential Manager or Linux Secret Service. With that
option off, the key lives in the connection for the current session. A profile
stores only the origin and installation ID. Disconnect clears the loaded
library and preview from the UI. To remove the saved local credential, select
**Forget saved key** from the connection screen after disconnecting.

The optional Rust diagnostic exercises the same direct read contract.

## Requirements

Provide the Oyster HTTPS origin, the enabled Prints installation ID, and a
revocable named App key with `apps.read` for that specific installation. The
installation must already have the appropriate library, Computer and media
permissions. Confirm the intended Computer in Oyster before connecting. The
selected Computer and library can change on the host, so integrations must
recheck that identity before caching or handing off owner assets.

If Prints asks for a gallery folder, sign in to Oyster in your browser and
open web Prints. Choose the intended Computer and gallery folder there, then
return to Prints Desktop and reconnect. A fresh local Oyster trial needs
this setup before the desktop can list its library.

The existing Loftwah host accepted a temporary Prints-only key. The desktop
opened six indexed outputs, exported an original whose bytes matched an
independent read of the same asset, and confirmed denial after revocation.
The installed Prints contract accepts root reads and lacks folder discovery.
Its model catalogue returned HTTP 502. The desktop opens the readable library
first and treats model discovery as optional; it offers web Prints for folder
navigation.

## Optional model discovery and local report

`listOutputs` opens the library on its own. `listModels` then runs on a
separate request lane with its own pending, error and retry state. While it is
pending, outputs can be selected, paged, previewed, compared offline and
exported. A temporary, malformed or permission-limited catalogue reply keeps
the library usable and offers **Retry model discovery** without changing the
page or selected output. Disconnect, confirmed revocation and a changed
installation invalidate both lanes and the export-dialog guard; a late reply
from an earlier connection is ignored, without claiming its transport was
cancelled.

Each failure keeps a small, reviewable **Connection report** beside the
catalogue. **Copy connection report** writes the same text shown on screen to
the clipboard for the existing bug form. It carries the app version and the
build's `PRINTS_SOURCE_REVISION` when supplied, OS and architecture, the typed
operation and method, a bounded HTTP status or failure category, an allowlisted
refusal code, elapsed milliseconds, any observed compatibility fallback, and a
bounded causal trail with opaque local correlation IDs. Keys, headers, bodies,
host names, paths, Computer and installation IDs, artwork and prompts are
never read into it. Copying is local: nothing is sent upstream or filed
automatically.

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
| `listFolders` | Browse folders in the selected library | `computer.list` |
| `getOutput` | Reconfirm the selected asset's identity around export | `computer.list` |
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
To verify a new installation, confirm model and library access, compare the
retrieved bytes against the same original, and verify denial after revocation.
Check the selected Computer before saving a connection.

Read-only transport tests use an isolated loopback server with fictional
responses. Run them with:

```sh
cargo test --locked --features oyster-reader --lib
```

## Creation

Choose **Create in web Prints** to open the upstream installation and use its
normal sign-in and creation flow. This handoff shares the installation URL.
The native reader uses named App keys, which the supported Oyster release
authenticates as Team-key actors. The supported Prints image, video and audio
generation handlers decline that actor type.

Native creation needs an upstream-supported scoped member or agent flow,
followed by an authorised live test for price, dispatch, retained output and
same-request recovery. Cloudflare infrastructure, Workers AI and external
inference have separate usage and billing.

- [Oyster's named-key authentication](https://github.com/fungi-computer/oyster/blob/3784e3071f10af40f2d179220e954da536b585d7/src/auth.ts#L256-L276)
- [Prints generation admission](https://github.com/fungi-computer/prints/blob/3475e6e4075fa858a908d58eeb9a67595bd1f373/backend/methods.ts#L1060-L1067)
