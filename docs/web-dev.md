# Browser terminal for development

Blackpepper's browser harness reuses `ttyd`: a PTY-backed web terminal with an
embedded xterm.js client. It streams terminal bytes, input, and resize events;
it does not stream screenshots or add a second Blackpepper terminal model.

## Run it

Close any running `bp-dev` (production `bp` may stay open), then run from the repository:

```bash
scripts/web-dev.sh
```

The script installs the exact source build with `scripts/setup.sh`, then prints
a URL such as:

```text
http://bp:0123456789abcdef0123456789abcdef@127.0.0.1:7681/bp-dev-fedcba9876543210fedcba9876543210/
```

Open that exact URL. The first browser connection owns the one-shot PTY; when
it disconnects, ttyd terminates the child and exits. A different fixed port can
be selected with `--port PORT`. Use `--skip-build` to exercise the already
installed development bundle.

The harness resolves `bp-dev` to its immutable current bundle before starting
the server. Reinstalling another source build while the server waits for its
first browser cannot change which binary that server launches.

Zellij copy mode emits an OSC 52 clipboard write. Blackpepper validates and
normalizes that write before offering it to the outer terminal. ttyd 1.7.7's
embedded xterm.js 5.4 does not handle OSC 52 itself, so the harness adds a
small local clipboard bridge to ttyd's own page. The bridge first asks the
browser to write the clipboard. If browser policy requires a user gesture, a
visible **Copy** button appears; the selected text stays in page memory and is
not shown in the banner.

## Safety boundary

The browser has shell-equivalent access, so this harness is deliberately more
restricted than a general web server:

- It always binds literal `127.0.0.1`; there is no interface option.
- Every launch generates independent 128-bit values for HTTP Basic
  authentication and the base path. The authenticated URL carries the
  one-shot password so browser controls do not stop at a login prompt.
- Basic authentication is the browser-facing access control. ttyd also
  receives its same-origin, random-base-path, one-client, and one-shot options
  as defense-in-depth.
- Browser-supplied command arguments, IPv6 binding, and public binding are not
  enabled.
- The page loads ttyd's embedded assets. It does not load scripts, fonts, or
  other content from a CDN. The clipboard bridge is a repository fixture
  injected into a private, checksum-keyed cache of ttyd's own embedded page.
- The child is the exact absolute `bp-dev` bundle path, never a shell command.

The bridge accepts only normalized system/primary clipboard writes (both map
to the browser's one clipboard), limits decoded text to 1 MiB, rejects invalid
UTF-8, and never answers OSC 52 clipboard reads. A terminal program that can print arbitrary
escape sequences can already request a clipboard write; the bridge does not
expand that trust boundary to clipboard reads.

ttyd 1.7.7's base-path and Origin checks are not treated as authentication:
its WebSocket path comparison accepts prefixes, and its Origin check does not
reliably reject a hostile handshake. Blackpepper therefore requires the
independent Basic-auth secret. ttyd runs at log level 3 so its notice-level
configuration output cannot print that credential.

The secret remains in ttyd's process argv. Linux and macOS can expose argv to
other local Unix users, who can also reach another user's loopback listener.
This harness therefore assumes a single-user machine or mutually trusted local
users; Basic auth protects against hostile browser origins and DNS rebinding,
not an untrusted account on the same host. Do not run it on a shared multi-user
server.

Plain HTTP is acceptable only inside this loopback boundary. To inspect a
development server on another machine, keep ttyd bound there and forward it:

```bash
ssh -L 7681:127.0.0.1:7681 development-host
```

Then open the printed authenticated URL on local port 7681. Do not expose the
ttyd port on a LAN or public interface; that requires a separately designed
authenticated TLS service. This follows [xterm.js's warning that a browser
terminal grants its page and JavaScript shell-level power](https://xtermjs.org/docs/guides/security/).

The development-channel per-user singleton still applies. The harness does
not create an isolated XDG state tree: if another `bp-dev` owns that channel,
the browser shows its PID and exits. Production `bp` has a separate singleton,
while both channels keep the same stable host/workspace/session registry and
lifecycle locks. Provider event stores are channel-specific. This lets `bp`
host work on Blackpepper while the browser exercises `bp-dev` without allowing
same-workspace mutations to bypass coordination.

## ttyd and browser behavior

The harness accepts a system ttyd whose output is exactly
`ttyd version 1.7.7` or starts with `ttyd version 1.7.7-`. That includes the
upstream `1.7.7-40e79c7` binary and Homebrew's `1.7.7-unknown` build while still
rejecting other versions. On Linux x86_64 and arm64, an absent or incompatible
system ttyd makes the harness fall back to the exact upstream
`1.7.7-40e79c7` release in `target/dev-tools/`; the script verifies its pinned
SHA-256 and version identity before publishing it atomically. The ignored cache
is never installed or packaged with Blackpepper. Upstream does not publish
macOS release binaries, so macOS requires a compatible 1.7.7 system ttyd; the
script gives the `brew install ttyd` action when it is absent.

The client uses ttyd's DOM renderer and xterm.js screen-reader mode so browser
automation and accessibility inspection can observe the TUI as well as send
keys and resize it. The page enables xterm.js's parser API only so the local
clipboard bridge can consume OSC 52. File-transfer and image addons default to
disabled. These are authenticated-client preferences, not a security boundary:
ttyd allows an authenticated user to override client preferences with URL
query parameters.

`127.0.0.1` is a browser secure context, but clipboard policy still varies.
The tab may need focus, site clipboard permission, or a direct click. The
bridge therefore does not claim success when `navigator.clipboard.writeText`
rejects: it keeps a visible retry button. If the retry is also denied, grant
clipboard permission for the page or use browser text selection. Automated
browser smoke tests can prove that the bridge is installed and receives OSC
52; they cannot prove an end-user permission grant.

Visual Studio Code uses the same broad design—xterm.js connected to a
pseudoterminal—but does not define Blackpepper's wire protocol. See
[VS Code's terminal notes](https://code.visualstudio.com/docs/terminal/advanced)
and [ttyd's options](https://github.com/tsl0922/ttyd#usage).

One known difference is intentional: ttyd 1.7.7 embeds xterm.js 5.4, while
xterm.js 6 added native OSC 52 support. The local bridge covers clipboard
writes without replacing ttyd; use this harness for interactive TUI
inspection, not as proof of other xterm.js 6-specific behavior. See the
[xterm.js releases](https://github.com/xtermjs/xterm.js/releases) and
[ttyd source](https://github.com/tsl0922/ttyd/tree/1.7.7).

## Validate it

The non-network test uses fake binaries to verify the complete ttyd argv,
loopback and one-client restrictions, independent authentication and path
entropy, immutable `bp-dev` path, private custom-index injection, installed
clipboard bridge, system-version compatibility, macOS guidance, log redaction
level, Homebrew compatibility, wrong-version rejection, and managed-download
checksum failure:

```bash
scripts/test-web-dev.sh
bash -n scripts/web-dev.sh scripts/test-web-dev.sh scripts/fixtures/web-dev/*.sh
shellcheck -x scripts/web-dev.sh scripts/test-web-dev.sh scripts/fixtures/web-dev/*.sh
```
