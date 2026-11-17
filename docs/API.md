# Zebra automation API

Zebra can be driven by programs the way Playwright drives Chrome. Every
change a person can make in the window can also be made through the API,
and the person keeps working in the same window while an agent edits. The
API is off unless you turn it on.

* **Transport:** JSON-RPC 2.0. Each message is one JSON object on its own line, sent over a Unix domain socket.
* **Clients:** `zebractl` (a command-line tool), `zebractl mcp` (an MCP server for Claude Code and other MCP clients), or any program that can open a Unix socket.
* **Rules for agents:** an agent never moves the person's camera, changes their selection or takes keyboard focus, unless the person grants it `full` permission. Agent edits show up in the person's undo history under the agent's name. The person can pause agents at any time, and the person always wins a conflict.

---

## 1. Turning it on

**For people:** click **AI: Off** in Zebra's toolbar and switch on **Let AI
agents control Zebra**. That's it; no launch flags, no terminal. The choice is
remembered. Only the person can switch it on: there is deliberately no API
method, flag in a file, or menu action an agent can reach to enable it. An
agent can switch it off with the `disconnect` method (`zebractl disconnect`).
Off means no socket and no discovery file, so nothing can find Zebra.

**For agents:** `zebractl` ships inside the app at
`/Applications/Zebra.app/Contents/MacOS/zebractl`, so it works without
installing anything. Start with `zebractl --help`, then `zebractl describe`.
If Zebra isn't open or the switch is off, every command fails with a message
asking the person to switch it on.

**For developers:**

| How | What happens |
|---|---|
| `zebra --api [file.zebra]` | Opens the normal window with AI control already switched on. |
| `zebra --headless --api [file]` | Runs with no window and the same API. Good for generating decks and for CI. The process prints `ZEBRA_API_READY {...}` on stdout when it is ready. |
| `--api-socket <path>` | Uses this socket path instead of the default one. |
| `--agent-permission observe\|edit\|full` | Sets the starting permission. The default is `edit`. The person can change it from the AI button in the toolbar. |

### Permissions

| Level | What the agent can do |
|---|---|
| `observe` | Read state and render images. It cannot make changes. |
| `edit` (default) | Make undoable document edits. It gets its **own** camera and selection, which are drawn dashed and violet. Present-mode and tool commands are refused. |
| `full` | Everything in `edit`, plus driving the visible camera, selection and present mode, and `open`/`save`. |

`zebractl mcp --spawn-headless` always starts its private instance with `full` permission, because nobody else is using that instance.

### Discovery (like Chrome's `DevToolsActivePort`)

Each instance with the API enabled writes a discovery file when it starts and removes it when it exits:

```
~/Library/Application Support/Zebra/instances/<pid>.json      (macOS, mode 0600)
~/.local/share/Zebra/instances/<pid>.json                      (Linux)
```

```json
{ "pid": 4242, "socket": "/var/folders/…/T/zebra-501/4242.sock",
  "token": "9f2c…", "version": "0.1.0", "api_version": 1,
  "started_at": 1763380000, "headless": false }
```

Mac App Store builds are sandboxed, so the app's own Application Support and
temp folders sit inside its container where the bundled `zebractl` (which
must also be sandboxed) can't see them. Both binaries therefore share an App
Group, and a sandboxed app writes its discovery file and socket there:

```
~/Library/Group Containers/<TEAMID>.app.zebra/Zebra/instances/<pid>.json
~/Library/Group Containers/<TEAMID>.app.zebra/s/<pid>.sock
```

Clients search, in order: `$ZEBRA_DATA_DIR`, the App Group, the normal data
directory above, and the app's own container. The group id and bundle id are
placeholders (`APP_GROUP_ID`, `BUNDLE_ID` in `zebra-api/src/discovery.rs`)
until the app is named and signed. `ZEBRA_GROUP_DIR` overrides the group path
for testing.

Set `ZEBRA_DATA_DIR` to replace the whole `…/Zebra` directory. The tests do this.
Clients ignore a file, and delete it, when its pid is no longer running.
The socket lives in `$TMPDIR/zebra-<uid>/` with directory mode 0700 and socket mode 0600.
If that path would be longer than macOS's 104-byte `sun_path` limit, the socket goes in `/tmp/zebra-<uid>/` instead.

### Authentication

The first call on a connection must be `hello`. The token is in the discovery file.

```json
→ {"jsonrpc":"2.0","id":1,"method":"hello","params":{"name":"Claude","token":"9f2c…"}}
← {"jsonrpc":"2.0","id":1,"result":{"server":"zebra","version":"0.1.0","api_version":1,"client_id":3,
     "name":"Claude","methods":[…],"events":[…],"next":"call `describe` for an outline, …"}}
```

The `name` is how the person sees this agent. It appears in the status bar
and in history labels such as "Claude: Add agenda". Every other method,
except `ping`, fails with `unauthenticated` until `hello` succeeds.

---

## 2. Methods

`params` is always an object. Where this section shows only a `result`, the
call takes no params.

### Reading

| Method | Params | Result |
|---|---|---|
| `ping` | – | `"pong"` (allowed before `hello`) |
| `describe` | – | `{"outline": "…text…"}`: a compact outline of the presentation (start here) |
| `state` | – | mode, step, selection, agent_selection, camera, agent_camera, revision, dirty, file, agent_permission, agent_paused, human_editing, can_undo, … |
| `query` | `{"query": {"q": …}}` (or `{"query": "path"}`) | depends on the query; see the reference below |
| `commands` | – | the catalogue: every command, view command and query, each with its description, params, an example and its JSON schema |
| `schema` | – | full JSON Schemas for `Command`, `ViewCommand`, `Query` and capture targets, plus the event and method lists |
| `methods` | – | the method names |

```json
→ {"jsonrpc":"2.0","id":2,"method":"query","params":{"query":{"q":"elements","kind":"frame"}}}
← {"jsonrpc":"2.0","id":2,"result":{"elements":[{"id":"f1","type":"frame","x":0,"y":0,…}]}}
```

### Editing (undoable)

| Method | Params | Result |
|---|---|---|
| `apply` | `{"command": {"op": …}, "label"?: "…", "if_revision"?: n}` (a `label` makes it its own undo step with that name) | `{"revision", "created"?, "changed"?, "deleted"?}` |
| `batch` | `{"commands": [ … ], "label"?: "…", "if_revision"?: n}` | the same; **atomic** (all or nothing) and **one undo step** named `label` |
| `begin` | `{"label": "Add timeline"}` | starts a transaction: each following `apply` takes effect immediately, and the whole transaction becomes one undo step |
| `commit` / `abort` | – | ends the transaction; `abort` restores the document to how it was at `begin` |
| `undo` / `redo` | – | `{"undone": "label" \| null, "revision"}` |

```json
→ {"jsonrpc":"2.0","id":3,"method":"batch","params":{"label":"Add intro","commands":[
     {"op":"add_frame","id":"intro","x":0,"y":0,"name":"Intro","add_to_path":true},
     {"op":"add_text","text":"Hello","style":"title","place":{"relative_to":"intro","side":"inside"}}]}}
← {"jsonrpc":"2.0","id":3,"result":{"revision":7,"created":["intro","t1"]}}
```

* `add_image` accepts `"path": "/abs/file.png"` in place of `data_base64`. The server reads the file.
* `if_revision` gives optimistic concurrency. If the document has moved past that revision, the call fails with `stale_revision`.
* An explicitly labelled `apply`, `batch` or transaction is always its own undo step. Single `apply` calls from the same agent that arrive less than 2 s apart are grouped into one step.
* An agent that disconnects while its transaction is open has that transaction aborted.

### View: camera, selection, presenting (not undoable)

| Method | Params | Result |
|---|---|---|
| `view` | `{"command": {"op": "zoom_to", "id": "f1"}}` | depends on the command |
| `wait` | `{"until": "idle" \| {"step": n} \| {"revision": n} \| {"mode": "present"}, "timeout_ms"?: 10000}` | `{"ok", "revision", "mode", "step", "build", "camera"}` or the error `timeout` |

`idle` means no camera flight is running and the person is not in the middle of a gesture.
`{"step": n}` means the app is presenting and the camera has finished flying to step `n`.

### Pictures and files

| Method | Params | Result |
|---|---|---|
| `capture` | `{"target"?, "width"?, "height"?, "path"?}` | `{"format":"png","width","height","target","data_base64"}`, or `{"path","bytes",…}` when `path` is given |
| `export` | `{"format": "pdf"\|"png"\|"json"\|"zebra", "path"?, "target"?, "width"?, "height"?}` | `{"path","bytes","mime"}`, or the data inline when no `path` is given (`json` returns `{"document": …}`) |
| `open` | `{"path": "/abs/deck.zebra", "force"?: false}` | needs `full`; fails with `unsaved_changes` unless `force` is set |
| `save` | `{"path"?: "/abs/deck.zebra"}` | needs `full`; with no `path` it saves to the current file |
| `disconnect` | – | switches AI control off in the app (the person must switch it back on); in headless mode it quits |
| `quit` | – | headless instances only |

Capture targets:

| Target | What it renders |
|---|---|
| `"viewport"` | the person's window |
| `"agent_view"` | your private camera; the default for `edit`/`observe` agents |
| `"overview"` | everything |
| `{"step": n, "build"?: k}` | path step `n`, 0-based |
| `{"element": "f1"}` | one element |
| `{"camera": {cx, cy, zoom, rotation}}` | an explicit camera |

Captures render off-screen and **never** move the person's camera.
File paths must be absolute, because the app's working directory is not
yours. `zebractl` converts relative paths for you.

### Events

```json
→ {"jsonrpc":"2.0","id":9,"method":"subscribe","params":{"events":["document_changed","step_changed"]}}
← {"jsonrpc":"2.0","id":9,"result":{"subscribed":["document_changed","step_changed"]}}
← {"jsonrpc":"2.0","method":"event","params":{"event":"document_changed","revision":8,
     "origin":{"kind":"human"},"label":"Move","ids":["t1"]}}
```

The available events are:

| Event | Fields |
|---|---|
| `document_changed` | revision, origin (`{"kind":"human"}` or `{"kind":"agent","name":…}`), label (as the person reads it, in Zebra's language), ids (created, changed and deleted) |
| `selection_changed` | selection |
| `mode_changed` | mode |
| `step_changed` | step, build |
| `agents_paused` | paused |
| `saved` | path |

`subscribe {}` with no `events` subscribes to everything, and `unsubscribe` stops events.
Events from a call reach subscribers before that call's response.

---

## 3. Errors

Errors are JSON-RPC errors whose `data` explains how to fix the problem:

```json
{"code":-32602,"message":"bad command: unknown variant `add_txet`, expected one of `add_element`, `add_text`, …",
 "data":{"code":"invalid_params","hint":"call `commands` for every op with an example","valid_values":["add_element","add_text",…]}}
```

| JSON-RPC code | `data.code` | Meaning / what to do |
|---|---|---|
| -32700 | `parse_error` | The line is not valid JSON. |
| -32600 | `invalid_request` | The `method` is missing, or the message was a batch array. Send one request per line. |
| -32601 | `method_not_found` | `valid_values` lists the methods. The hint tells you when you sent a command op as a method name. |
| -32602 | `invalid_params` | Bad params or a bad command. The message contains the serde error and the hint has an example. |
| -32001 | `unauthenticated` | Call `hello` with the token from the discovery file. |
| -32000 | `not_found` | Unknown element id. `valid_values` holds the nearest ids. |
| -32000 | `busy` | The person was mid-gesture for more than 5 s, or another client has a transaction open. Retry. |
| -32000 | `paused` | The person paused agents. Wait for `agents_paused {paused:false}`. |
| -32000 | `presenting` | The person is presenting and edits are held. Wait for `mode_changed`. |
| -32000 | `permission` | The call needs a higher permission level. The hint says which one. |
| -32000 | `stale_revision` | The `if_revision` check failed. Re-read and retry. |
| -32000 | `no_transaction` | `commit` or `abort` without a matching `begin`. |
| -32000 | `unsaved_changes` | `open` would discard the person's work. Pass `force`. |
| -32000 | `timeout` | A `wait` condition was not met in time. |
| -32000 | `io`, `bad_file`, `no_path`, `unimplemented` | File and renderer problems. |

**"Human wins":** while the person is dragging or typing, agent edits are
queued for up to 5 s. If the gesture is still going after that, the call
fails with `busy`. Reads (`describe`, `state`, `query`, `capture`) are
never blocked.

---

## 4. `zebractl`

```
zebractl instances                          # running instances
zebractl state | describe
zebractl query '{"q":"elements","kind":"text"}'     # or: zebractl query path
zebractl apply '{"op":"set_theme","theme":"night"}'
zebractl apply -f build.json --label "Build deck"   # a JSON array = one undo step
zebractl view '{"op":"zoom_to","id":"f2"}'
zebractl add frame --x 0 --y 0 --name Intro --path
zebractl add text "Hello" --inside f1 --style title
zebractl add text "Note" --x 100 --y -50
zebractl add image ./logo.png --x 0 --y 400 --width 300
zebractl add shape arrow --x 500 --y 0 --w 300 --h 60
zebractl move f1 --dx 200 --dy 0 | delete t3 s1 | select t1 | set-text t1 "New"
zebractl undo | redo
zebractl present [step] | next | prev | stop
zebractl screenshot --step 2 -o step2.png --width 1600 --height 1000   # or --element f1 | --overview | --viewport
zebractl export pdf -o deck.pdf        # png | json | zebra
zebractl open deck.zebra [--force] | save [path]
zebractl events --follow [--filter document_changed,step_changed]   # JSON lines
zebractl wait idle | step 3 | revision 12 | mode present  [--timeout-ms 10000]
zebractl raw '{"method":"state"}'
zebractl commands [--markdown | --json]
zebractl mcp [--spawn-headless [--file deck.zebra] [--zebra-bin path]]
```

Global options:

| Option | Meaning |
|---|---|
| `--instance <pid>` | connect to the instance with this pid |
| `--socket <path>` | connect to this socket |
| `--token <t>` | auth token; also read from the `ZEBRA_TOKEN` env var |
| `--name <client name>` | the name the person sees; default `zebractl` |

Results are printed as JSON. Errors go to stderr with their code, hint and
valid values, and set exit status 1.

---

## 5. MCP (Claude Code, Claude Desktop, other MCP clients)

```sh
# attach to the Zebra window you have open (switch on AI control in the app)
claude mcp add zebra -- zebractl mcp

# or give the agent its own private, invisible document
claude mcp add zebra-headless -- zebractl mcp --spawn-headless
```

`zebractl mcp` is a stdio MCP server that supports protocol version
`2025-06-18`. It connects to Zebra the first time a tool is called, and
reconnects if Zebra restarts. In MCP the agent's default name is `Claude`;
change it with `--name`. It offers eight tools. Their schemas come from the
same Rust types as the API.

| Tool | Does |
|---|---|
| `describe` | Returns the text outline. Start here. |
| `get_state` | Returns the session state. |
| `apply_commands` | `{commands: [...], label?}`: atomic, one undo step. |
| `view` | `{command, wait?}`: camera, selection, presenting. Waits until the camera is idle. |
| `query` | `{query: {q: …}}`: read-only queries. |
| `capture` | `{target?, width?, height?}`: returns an MCP **image** content block. |
| `undo` | `{redo?, steps?}`. |
| `list_commands` | The full command reference as compact text. |

The server's `instructions` teach the model the essentials. The ids are
short (`f1`, `t3`). Coordinates are world units. x and y are the centre of
an element, and y grows downward. Frames carry their contents when they
move. Path steps are 0-based. Theme colour slots look like `theme:accent`.

---

## 6. Walkthrough: a 3-step presentation

```sh
zebra --headless --api --agent-permission full &      # or use the app with AI control switched on

cat > deck.json <<'EOF'
[
  {"op":"set_title","title":"Why zebras?"},
  {"op":"set_theme","theme":"classic"},
  {"op":"add_frame","id":"big","x":0,"y":0,"w":3000,"h":2000,"style":"invisible","name":"Overview"},
  {"op":"add_text","text":"Why zebras?","style":"title","x":0,"y":-700,"scale":3},

  {"op":"add_frame","id":"s1","x":-900,"y":200,"style":"circle","name":"Stripes","add_to_path":true},
  {"op":"add_text","text":"Stripes confuse flies","place":{"relative_to":"s1","side":"inside"}},

  {"op":"add_frame","id":"s2","x":0,"y":200,"style":"bracket","name":"Herds","add_to_path":true},
  {"op":"add_text","text":"Herds\nmove together","style":"subtitle","place":{"relative_to":"s2","side":"inside"}},

  {"op":"add_frame","id":"s3","x":900,"y":200,"w":400,"h":300,"rotation":-10,"name":"Detail","add_to_path":true},
  {"op":"add_text","text":"No two are alike","place":{"relative_to":"s3","side":"inside"}},
  {"op":"add_shape","shape":"arrow","x":-450,"y":200,"w":250,"h":60,"fill":"theme:accent"}
]
EOF

zebractl apply -f deck.json --label "Build deck"   # one undo step
zebractl describe                                  # check ids, text and the path
zebractl query lint                                # overlaps, unreadable text
zebractl screenshot --step 2 -o step2.png          # look at what the audience will see
zebractl present 0 && zebractl wait step 0
zebractl next && zebractl wait step 1
zebractl export pdf -o why-zebras.pdf
zebractl save ~/why-zebras.zebra
```

From an MCP client the same work is one `apply_commands` call with
`label: "Build deck"`, then `capture {"target":{"step":2}}` to check the
result.

---

## 7. Command reference

This section is generated by `zebractl commands --markdown`. The same data
is returned by the `commands` method and the `list_commands` MCP tool.

<!-- BEGIN GENERATED -->
### Document commands

Send with `apply {"command": ...}` or inside `batch {"commands": [...]}`. Undoable.

| op | params | description | example |
|---|---|---|---|
| `add_element` | element: NewElement | Add any element. Most callers prefer the `add_*` helpers. | `{"op":"add_element","element":{"x":0,"y":0,"kind":{"type":"shape","shape":"ellipse"},"w":150,"h":150}}` |
| `add_text` | id?: string, place?: Placement, scale?: number, style?: TextStyle, text: string, x?: number, y?: number | Double-click-to-write: add a text box. `x`,`y` is the centre. If `scale` is omitted the text is sized to read well at the given `place` (or at scale 1). | `{"op":"add_text","text":"Hello world","x":0,"y":-200,"style":"title"}` |
| `add_frame` | add_to_path?: boolean, h?: number, id?: string, name?: string, rotation?: number, style?: FrameStyle, w?: number, x?: number, y?: number | Add a frame. `w`,`h` are world units at scale 1 (default 800x600). | `{"op":"add_frame","x":0,"y":0,"w":800,"h":600,"style":"bracket","name":"Intro","add_to_path":true}` |
| `add_shape` | fill?: string, h?: number, id?: string, rotation?: number, shape: ShapeKind, stroke?: string, w?: number, x?: number, y?: number |  | `{"op":"add_shape","shape":"arrow","x":500,"y":0,"w":300,"h":60,"fill":"theme:accent"}` |
| `add_image` | alt?: string, data_base64: string, id?: string, width?: number, x?: number, y?: number (or path: string, an absolute file path, instead of data_base64) | Insert an image from bytes (base64). The API also accepts `path`. | `{"op":"add_image","path":"/Users/me/logo.png","x":0,"y":300,"width":400,"alt":"Logo"}` |
| `add_ink` | color?: string, highlighter?: boolean, points: number[][], width?: number | Freehand pen / highlighter stroke in world coordinates. | `{"op":"add_ink","points":[[0,0],[50,40],[120,10]],"color":"theme:accent","width":6}` |
| `update` | id: string, patch: ElementPatch | Change properties of one element. Only given fields change. | `{"op":"update","id":"t1","patch":{"text":"New text","bold":true,"color":"#cc3333"}}` |
| `move` | dx: number, dy: number, ids: string[], with_contents?: boolean | Move elements by a world delta. Frames carry their contents. | `{"op":"move","ids":["f1"],"dx":200,"dy":0}` |
| `scale` | about?: array, factor: number, ids: string[], with_contents?: boolean | Scale elements about a world point (default: their common centre). | `{"op":"scale","ids":["t1"],"factor":0.5}` |
| `rotate` | about?: array, degrees: number, ids: string[], with_contents?: boolean | Rotate elements by degrees (clockwise) about a point. | `{"op":"rotate","ids":["f2"],"degrees":-15}` |
| `delete` | ids: string[] |  | `{"op":"delete","ids":["s3"]}` |
| `duplicate` | dx?: number, dy?: number, ids: string[] | Copy elements (frames bring their contents). New ids are returned. | `{"op":"duplicate","ids":["f1"],"dx":1000,"dy":0}` |
| `z_order` | ids: string[], order: ZOrderOp |  | `{"op":"z_order","ids":["i1"],"order":"back"}` |
| `wrap_in_frame` | add_to_path?: boolean, ids: string[], padding?: number, style?: FrameStyle | Draw a frame around existing elements. | `{"op":"wrap_in_frame","ids":["t1","t2"],"style":"circle","add_to_path":true}` |
| `arrange` | gap?: number, ids: string[], layout: Layout | Arrange elements in a row, column, grid or circle. | `{"op":"arrange","ids":["f1","f2","f3"],"layout":"row"}` |
| `align` | edge: AlignEdge, ids: string[] | Align elements (snapping helper). | `{"op":"align","ids":["t1","t2"],"edge":"left"}` |
| `add_step` | builds?: string[], index?: integer, notes?: string, target: StepTarget | Insert a step (default: at the end). | `{"op":"add_step","target":{"kind":"element","id":"f1"},"builds":["t2","t3"],"notes":"Say hi"}` |
| `remove_step` | index: integer |  | `{"op":"remove_step","index":2}` |
| `move_step` | from: integer, to: integer |  | `{"op":"move_step","from":3,"to":0}` |
| `update_step` | builds?: array, duration?: number, index: integer, notes?: string, target?: StepTarget |  | `{"op":"update_step","index":0,"notes":"Open with a question","duration":2.5}` |
| `clear_path` |  |  | `{"op":"clear_path"}` |
| `auto_path` |  | Replace the path with every frame in reading order (top-left to bottom-right, larger frames first). | `{"op":"auto_path"}` |
| `set_theme` | theme: string |  | `{"op":"set_theme","theme":"chalkboard"}` |
| `set_title` | title: string |  | `{"op":"set_title","title":"Quarterly review"}` |

### View commands

Send with `view {"command": ...}`. Session state, not undoable.

| op | params | description | example |
|---|---|---|---|
| `set_camera` | animate?: boolean, camera: Camera |  | `{"op":"set_camera","camera":{"cx":0,"cy":0,"zoom":0.5,"rotation":0},"animate":true}` |
| `zoom` | anchor?: array, factor: number | Zoom by a factor around a screen point (default: viewport centre). | `{"op":"zoom","factor":2}` |
| `pan` | dx: number, dy: number | Pan by a screen-space delta in points. | `{"op":"pan","dx":-200,"dy":0}` |
| `zoom_to` | animate?: boolean, id: string | Fly to fit an element (frames: match their rotation). | `{"op":"zoom_to","id":"f1"}` |
| `fit_all` | animate?: boolean |  | `{"op":"fit_all"}` |
| `zoom_to_selection` |  |  | `{"op":"zoom_to_selection"}` |
| `select` | ids: string[] |  | `{"op":"select","ids":["t1","t2"]}` |
| `add_to_selection` | ids: string[] |  | `{"op":"add_to_selection","ids":["s1"]}` |
| `clear_selection` |  |  | `{"op":"clear_selection"}` |
| `select_all` |  |  | `{"op":"select_all"}` |
| `set_tool` | tool: Tool |  | `{"op":"set_tool","tool":"pen"}` |
| `set_frame_style` | style: FrameStyle |  | `{"op":"set_frame_style","style":"circle"}` |
| `set_mode` | mode: Mode |  | `{"op":"set_mode","mode":"path_edit"}` |
| `present` | step?: integer | Enter present mode at a step (default: first step). | `{"op":"present","step":0}` |
| `next` |  | Next build or step. | `{"op":"next"}` |
| `prev` |  |  | `{"op":"prev"}` |
| `goto_step` | step: integer |  | `{"op":"goto_step","step":2}` |
| `overview` |  | Fly to the overview while presenting. | `{"op":"overview"}` |
| `blank` | blank?: Blank |  | `{"op":"blank","blank":"black"}` |
| `exit_present` |  |  | `{"op":"exit_present"}` |
| `set_viewport` | height: number, width: number |  | `{"op":"set_viewport","width":1280,"height":800}` |
| `set_reduce_motion` | on: boolean |  | `{"op":"set_reduce_motion","on":true}` |
| `follow_agent` | on: boolean |  | `{"op":"follow_agent","on":true}` |
| `pause_agents` | paused: boolean |  | `{"op":"pause_agents","paused":true}` |
| `set_agent_permission` | permission: Permission |  | `{"op":"set_agent_permission","permission":"full"}` |

### Queries

Send with `query {"query": ...}`. Read-only.

| op | params | description | example |
|---|---|---|---|
| `describe` |  | Compact text outline of the whole presentation (start here). | `{"q":"describe"}` |
| `state` |  | Mode, selection, camera, revision, step, tool, theme... | `{"q":"state"}` |
| `document` |  | Full document JSON (without image bytes). | `{"q":"document"}` |
| `element` | id: string |  | `{"q":"element","id":"t1"}` |
| `elements` | kind?: string, text?: string, visible?: boolean, within?: string | Filter elements. All filters are optional and combined with AND. | `{"q":"elements","kind":"text","within":"f1"}` |
| `hit_test` | x: number, y: number | Topmost element at a world point. | `{"q":"hit_test","x":120,"y":-40}` |
| `hit_test_screen` | x: number, y: number | Topmost element at a screen point of the human's window. | `{"q":"hit_test_screen","x":640,"y":400}` |
| `path` |  |  | `{"q":"path"}` |
| `themes` |  |  | `{"q":"themes"}` |
| `history` |  |  | `{"q":"history"}` |
| `lint` |  | Layout lint: overlaps, text unreadable at its step, empty steps... | `{"q":"lint"}` |

<!-- END GENERATED -->
