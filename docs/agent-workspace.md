# Work with a local agent in Prints

Open **Agent access** in the running app. **Enable read access** lets your chosen local agent read the brief and permitted image metadata. **Enable edit access** also lets it work on that brief, shortlist, selection and comparison. The grant covers this workspace. Use **Copy grant path** and share the resulting local path with the client you want to use. Keep the grant file private. Changing the access mode, revoking access or closing Prints ends that grant.

The archive is available to the agent. Linked artwork is visible when it is selected, shortlisted or in the comparison. The agent receives stable IDs and display titles. Exporting an original stays a deliberate action in the app.

## Use the packaged command

Use the executable inside the same package as your running app. On macOS that is `Prints Desktop.app/Contents/MacOS/prints-desktop`. Linux packages contain `prints-desktop`. Windows packages contain `Prints Desktop.exe`.

The executable accepts a JSON command on standard input and returns a JSON result. Point `PRINTS` at the executable and `GRANT` at the path copied from the app.

```sh
printf '%s\n' '{"id":"discover-1","request":{"op":"discover"}}' |
  "$PRINTS" workspace --grant "$GRANT"

printf '%s\n' '{"id":"read-1","request":{"op":"read"}}' |
  "$PRINTS" workspace --grant "$GRANT"
```

An edit uses a fresh command ID, the exact revision from the last read and an asset ID from the permitted metadata when needed. Keep revisions as strings.

```json
{"id":"brief-2","request":{"op":"edit_brief","expected_revision":"42","text":"Compare the lettering at 100%. Keep the square edition for the album tile."}}
```

The supported edits are `select`, `compare`, `edit_brief`, `keep`, `remove` and `reopen_handoff`. Select, keep and remove take an `asset` ID. Compare takes `first` and `second` IDs. The reopen operation returns to the draft in the running workspace. Select a known asset ID to return to it after reopening and granting access to the same workspace.

Read again after a conflict and ask before replacing a human edit. Reuse the same command ID only when retrying the same request. If a command result is uncertain, read the workspace to see what landed. Results describe the state of the running app. An accepted edit may report a failed local save. The native draft remains available to retry.

## Inspect the packaged UI

The packaged binary supports the Rune screenshot and element-tree commands. Use a new or empty dedicated directory if the replay needs to reopen the same state.

```sh
"$PRINTS" --verify-profile "$PROOF_DIR" --tree "$TREE" --snap "$IMAGE" --size 940x660
```

A tooling invocation without a verification directory uses temporary preferences. An explicit verification directory is separate from normal app preferences. Use fictional or public inputs when sharing a capture. Tooling profiles use session-only Oyster keys. [Rune tooling reference](https://github.com/anomalyco/rune/blob/e725e066548688ad1bb1f38ce64ae73f06ec0968/crates/rune/README.md#agent-tooling-headless-no-window-same-renderer-fixed-virtual-clock)
