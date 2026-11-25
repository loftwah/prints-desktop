# Native workspace

## Direction contract

**Thesis.** A photographic contact sheet you can work in. Let the image stay
large enough to see the light and surface while the working controls stay close.

**Own world.** Ink-violet shell, warm silver typography and one tangerine
selection colour. The light mode resembles a print laid out on an ivory table.

**Story.** Launch into the Loftwah Creative Archive, choose a print in the
contact strip, inspect its provenance and export the original. Compare the
LoftwahFM cover and release poster to see how one design serves two formats.
Afterlight offers a separate set of prints with briefs to carry into a draft.

**First viewport.** Compact left navigation, central image with Find above and
a caption beneath it, one contact strip and a slim right inspector. A thumbnail
selection changes the large print and its provenance together.

**Form.** Code-led, image-first desktop workspace with a pinned creative
direction from the brief. Native controls serve the artwork.

**Finish.** unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

## Lifecycle review

The [v0.4.1 source](https://github.com/loftwah/prints-desktop/pull/12) makes connection, paging and export transitions explicit. [OpenCode's native Rune architecture](https://github.com/anomalyco/opencode-rune/blob/a9f4d0251a1f99379492b2be09895160c6283d85/ARCHITECTURE.md) was the reference for separating feature states and keeping stale asynchronous results from becoming current UI. Prints retains its smaller workspace model.

| Area | Model and invariant |
| --- | --- |
| View and navigation | `Screen` and `Shelf` stay separate. Compare and Preview can return to the originating shelf without reselecting the library. |
| Local and bundled originals | Afterlight, archive editions and linked files have explicit source variants. A linked original keeps its path when its image is unavailable so Refresh and Locate can recover it. |
| Hosted library | An authorised library owns the client, visible page and selection. A typed operation owns connect, pagination, folder browse and transfer. A failed page load keeps the last successful page. |
| Native export | Choosing a path and writing original bytes are distinct states. Closing waits for an accepted write to finish. A stale hosted Save callback cannot write after its connection is replaced. |
| Find | Search words update on edits. Each image's searchable text is normalised once instead of on every render. |
| Simple view preferences | Theme, inspector visibility and remembering credentials are independent settings. A draft has one dirty bit that changes when its text changes. Rune owns image decode and latest-request delivery. |

An isolated Find measurement used 11 public assets and 50,000 mixed-query passes per trial. Across seven interleaved trials, the median filter time fell from 182.42 ms to 10.81 ms. This measures filtering work independently of application launch, rendering and filesystem I/O.
