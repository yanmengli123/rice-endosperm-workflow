# StickS3 Device Bridge

StickS3 Device Bridge is an experimental, opt-in Remote Access integration for
using a StickS3 as a physical Wisp Science status pet. Open **Settings → Remote
Access → StickS3 Device Bridge** to configure it.

The transport is designed as two explicit modes from the start:

- **Local network (LAN)** is implemented in the first release. StickS3 polls a
  concrete IPv4 address on the computer.
- **Relay server** is reserved for a later release. It will allow a device and
  computer on unrelated networks to meet through an authenticated intermediary.
  The UI shows this mode as planned, but no relay connection is made today.

Keeping the modes separate prevents a future relay configuration from silently
changing the LAN listener's exposure or reusing its address and port settings.

## When wired Ethernet and Wi-Fi can communicate

The computer does not need to use Wi-Fi. A StickS3 on Wi-Fi can reach a computer
on Ethernet when the network routes packets between their subnets and the
computer firewall permits the selected TCP port.

These descriptions are not equivalent:

- **Same router** often means the wired and wireless clients are routable, but
  guest Wi-Fi or client isolation can still block them.
- **Routable networks** means the network has an IP route and permits traffic
  from StickS3 to the computer's selected IPv4 address. The two devices may be
  in different private subnets.
- **Same public IP** only means both devices may share one Internet egress
  address through NAT. It does not make them mutually reachable and is not a
  substitute for LAN routing.

Do not expose the LAN listener with router port forwarding. Use the future relay
mode for out-of-LAN access once that mode is implemented.

## Find the computer's wired IPv4 address

On Windows:

1. Open PowerShell.
2. Run `Get-NetIPAddress -AddressFamily IPv4` or `ipconfig`.
3. Find the active **Ethernet** adapter and note its IPv4 address, for example
   `10.10.87.103`. Ignore `0.0.0.0`, loopback (`127.0.0.1`), disconnected
   adapters, VPN adapters, and virtual-machine adapters unless those are
   intentionally routed to StickS3.

On macOS:

1. Open System Settings → Network → Ethernet → Details → TCP/IP, or run
   `ipconfig getifaddr en0` in Terminal (the Ethernet interface name may differ).
2. Use the IPv4 address assigned to the active wired interface.

If unsure, ask the network administrator which computer address is reachable
from the StickS3 Wi-Fi network.

## Enable LAN mode

1. Open **Settings → Remote Access → StickS3 Device Bridge**.
2. Select **Local network (LAN)**. Relay server is visible but unavailable in
   this phase.
3. Enter one concrete IPv4 address belonging to this computer.
4. Keep the default port `18766`, or choose another port from 1 through 65535.
5. Enable Device Bridge and save.
6. Confirm that the state is **Listening** and copy the displayed listening URL
   into the StickS3 network-test configuration.
7. Generate a device token and copy it into the StickS3 configuration as the
   `X-Wisp-Device-Token` request header.

Device Bridge is disabled by default, including after upgrading an existing
installation. It never binds `0.0.0.0`; a specific IPv4 address is mandatory.
Its default port is `18766`. The separate Browser Bridge remains loopback-only
on `127.0.0.1:18765`.

If binding fails, Settings reports an error while the rest of Wisp Science
continues to work. Common causes are an address that is no longer assigned to
the computer or a port already used by another process.

## Firewall

The operating-system firewall may ask whether Wisp Science may accept incoming
connections. Permit TCP traffic only for the selected Device Bridge port and
the intended private network profile or source subnet. Do not create a broad
all-networks rule, and do not open Browser Bridge port `18765` to the LAN.

Network equipment can also block traffic. Check wireless client isolation,
guest VLAN rules, inter-VLAN access control lists, and routing between the
StickS3 Wi-Fi subnet and the computer's Ethernet subnet.

## HTTP protocol v1

The LAN base URL is `http://<selected-ipv4>:<port>`. Responses intentionally
exclude prompts, conversation text, commands, files, credentials, tool output,
and SQLite content.

### `GET /health`

This network-test endpoint is public and returns only:

```json
{
  "ok": true,
  "service": "wisp-device-bridge",
  "protocol": 1
}
```

### `GET /state`

Requires `X-Wisp-Device-Token` and returns the backend-owned physical-pet state:

```json
{
  "type": "pet_state",
  "state": "working",
  "project": "Wisp Science",
  "label": "Agent is working",
  "sessionId": "frame-id-or-null",
  "seq": 42,
  "updatedAt": 1785141209
}
```

State values are `idle`, `working`, `review`, `needs_user`, `done`, and
`failed`. With parallel sessions the display priority is:

`needs_user > failed > review > working > done > idle`

`seq` increases monotonically whenever the authoritative state changes.
`sessionId` lets the physical button focus the relevant desktop session.

### `GET /pet/manifest`

Requires `X-Wisp-Device-Token`. When the desktop Pet setting points to a valid
Codex-compatible v2 Pet package, this endpoint describes the frames available
to StickS3:

```json
{
  "type": "pet_manifest",
  "protocol": 1,
  "enabled": true,
  "id": "wispy",
  "displayName": "Wispy",
  "revision": "64-character-lowercase-sha256",
  "format": "png",
  "frameWidth": 120,
  "frameHeight": 130,
  "frameIntervalMs": 180,
  "frameCounts": {
    "idle": 7,
    "working": 6,
    "review": 6,
    "needs_user": 6,
    "done": 5,
    "failed": 8
  }
}
```

The revision covers the Pet manifest, atlas contents, validation data,
effective frame counts, identity, and StickS3 rendering parameters. It changes
whenever any StickS3-visible Pet output can change.

If the desktop Pet is disabled, unconfigured, or invalid, the endpoint still
returns HTTP 200 with `enabled: false`. It may include a short reason, but never
includes the configured Pet directory or another local path:

```json
{
  "type": "pet_manifest",
  "protocol": 1,
  "enabled": false,
  "reason": "Pet is disabled."
}
```

### `GET /pet/frame?revision=<revision>&state=<state>&frame=<index>`

Requires `X-Wisp-Device-Token`. A successful response is an immutable,
transparent `image/png` frame with exact dimensions `120×130` and an accurate
`Content-Length`. Wisp Science crops one `192×208` atlas cell and scales it at
the same aspect ratio. PNG and WebP v2 source atlases are supported.

The accepted states form a closed list:

| Bridge state | v2 atlas row | Row index | Default frame count |
|---|---|---:|---:|
| `idle` | `idle` | 0 | 7 |
| `working` | `running` | 7 | 6 |
| `review` | `review` | 8 | 6 |
| `needs_user` | `waiting` | 6 | 6 |
| `done` | `jumping` | 4 | 5 |
| `failed` | `failed` | 5 | 8 |

Valid `validation.json` cell data overrides the default frame counts in both
the manifest and frame-index validation. Unknown states and invalid or
out-of-range frame requests return HTTP 400 or 404. A stale revision returns
HTTP 409 so the device can fetch the manifest again. If no valid Pet is
available, the frame endpoint returns HTTP 404.

The endpoint accepts no path, filename, dimensions, or arbitrary file
parameter. Rendered frames are held in a bounded in-memory cache keyed by
revision, state, and frame; changing revision invalidates the previous cache.

### `POST /action`

Requires `X-Wisp-Device-Token`. Only three actions are accepted:

```json
{
  "id": "sticks3-123456",
  "action": "focus_session",
  "sessionId": "frame-id"
}
```

- `ping` records a bounded debug event and returns an acknowledgement.
- `focus_session` validates that the session and its project still exist,
  restores Wisp Science, and opens only that session's window.
- `acknowledge` clears a completed or failed physical-pet notification. It does
  not affect the Agent.

Unknown actions return HTTP 400. No action can submit a prompt, run a tool or
shell command, approve a request, or access SQLite directly.

### `GET /actions`

Requires `X-Wisp-Device-Token`. It returns at most the 50 most recent Device
Bridge action records for experimental diagnostics. The history is held in
bounded memory and is not a conversation transcript.

## Token handling

The first enable generates a random 256-bit pre-shared token when none exists.
Wisp Science stores it through the existing secret-storage path (the operating
system keyring in release builds), never in SQLite or normal logs. Authenticated
routes compare a token-derived fixed-size MAC rather than directly comparing
the supplied secret.

Use **Copy token** to configure StickS3. Treat the token like a password. Do not
paste it into issue reports or logs. **Rotate token** invalidates the previous
token immediately. **Revoke token** makes all authenticated requests fail until
a new token is generated.

## Stop or revoke device access

- Turn off **Enable Device Bridge** to revoke the in-memory token, stop the
  listener, release the port, and delete the stored token.
- Use **Revoke token** to invalidate authentication immediately while retaining
  the listener configuration.
- Remove any firewall rule if this computer should no longer accept StickS3
  traffic.

Changing settings or starting the service again stops the old listener before
the replacement starts. Application exit also stops the listener.

## Current limitations

- LAN mode uses HTTP polling; there is no WebSocket or mDNS discovery.
- Relay server mode is designed but not implemented or deployed.
- There is no cloud relay, router port-forwarding workflow, or automatic
  pairing.
- Pet resources use the configured local desktop Pet only; there is no remote
  Pet catalog or Pet upload endpoint.
- StickS3 cannot approve tools, submit prompts, run commands, or read full
  conversation content.
- One bounded action stream is provided for diagnostics; there is no complex
  multi-device orchestration.
- This repository does not modify or ship StickS3 firmware.

## Planned relay phase

The next phase should define a separately authenticated outbound relay client,
relay deployment and ownership, device registration/revocation, end-to-end
message authentication, replay protection, reconnect/backoff behavior, and
metadata-minimizing routing. It should preserve the same minimal pet-state and
action whitelist rather than tunnelling arbitrary HTTP, prompts, or tools.
