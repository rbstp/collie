# macOS VM and host Simulator testing

Run the test daemon in a disposable macOS guest and iOS Simulator on the Apple Silicon host. The harness uses its own local tailnet control, mock relay, identities and pairing. It does not require a Tailscale account, Cloudflare account or Apple signing credentials, and leaves the installed owner daemon and personal direct APNs configuration alone.

Two consecutive fresh `just vm-reset` and `just vm-test` runs passed at clean revision `90f747c7f66f3f7e0c328b2702bcc0a5c767f892`. Each reproved guest desktop capture and a Calculator accessibility click changing `0` to `6`, refusal of VM mode with an ordinary Release core, isolated VM identities, ordinary mutual-TLS pairing, encrypted mock push, and ordinary peer removal with a rejected follow-up. The prepared guest retained its Driver grants without another consent round. These are local integration results, not genuine App Attest or APNs delivery.

## Requirements and pinned inputs

Use the existing [build prerequisites](../install/requirements.html), including initialized submodules, Rust, Go, just, Xcode, XcodeGen, jq and Node with npm. The current harness checks for an Apple Silicon macOS 27 host and an available iOS 27 Simulator runtime. The exercised host is macOS 27.2; other host/runtime combinations have no acceptance result in this lane. Go is read from `/opt/homebrew/opt/go/bin/go`.

Reserve at least 80 GiB free for VM setup, plus separate space for Rust and Xcode build outputs and the existing Xcode/Simulator installation. The image download is 34.13 GB and its disk metadata reports 41.05 GB. The guest receives two virtual CPUs and 4 GiB RAM. Leave memory for Xcode and Simulator on the host too. These figures are inputs and a resource budget, not a measured maximum after builds.

| Input | Pin |
|---|---|
| Tart | `2.41.1`, archive SHA-256 `e680a734febccfce7d4e910c1f7f13fc3b00f19a181c945e2d746de3214c32c2` |
| macOS base | `ghcr.io/cirruslabs/macos-golden-gate-base@sha256:5302a9ea6cd68c059d00fa253fe75c8ebdf4e0dc8c45d4e136ecc6e7a4c02b70` |
| Guest CuaDriver | `0.36.0`, archive SHA-256 `4a0e229922a1ac6d6f59152e0653541920951afa89a3007e4ff5ffc024eae7a8` |

Guest boot reports macOS 27.0, build `26A428`. The harness downloads Tart into its own tool directory, verifies its signature and Gatekeeper assessment, and uses a separate `TART_HOME`. It installs the signed CuaDriver app only inside the guest and disables its telemetry. Driver control uses a guest `admin`-owned UNIX socket with mode `0600`, without a TCP listener. No global Tart installation, host Accessibility grant or host screen-recording grant is part of this procedure.

All owned state lives at `<repo-parent>/.codex/vms/collie-374`, shared by this repository's worktrees. Only one harness command may run at a time. The dedicated host Simulator is named **Collie VM test**; its exact identifier and VM ownership are recorded in `owned.json`. Treat this directory as private: it contains disposable authentication material, pairing state and test logs. Do not commit or share it.

## Set up the guest desktop once

From the repository checkout:

```sh
just setup
just vm-setup
```

The first setup downloads the pinned base, creates the dedicated iPhone 18 Pro Simulator, opens `collie-374-test`, and installs CuaDriver in the guest. Download and build prerequisites need network access. Existing unrelated VMs, Simulators, installed daemon state, Keychain items and real pairings are outside its ownership.

Log in to this disposable pinned guest with username `admin` and password `admin`. These are the public defaults of this CI image, not personal credentials. Complete the visible permission prompts inside that guest: grant CuaDriver **Accessibility** and **Screen & System Audio Recording**, then accept its relaunch. Complete any guest System Events or direct capture prompt when shown. These are guest permissions, not host permissions. Run:

```sh
just vm-desktop-ready
```

This verifies both grants attributed to the Driver daemon, clicks Calculator through its accessibility tree to prove a visible `0` to `6` change, lists guest apps and captures PNG screenshots. It saves private evidence under `desktop-proof`, stops the guest, and creates `collie-374-desktop`. That prepared baseline retains the guest permission decisions without run-specific relay, SSH or pairing credentials. A successful command prints `Guest desktop grants and capture verified; prepared baseline collie-374-desktop` and the evidence directory. The before/after Calculator screenshots and accessibility state accompany the permission and desktop evidence; the check does not claim every app can be controlled.

## Run the integration test

```sh
just vm-test
```

Keep this command in the foreground. Xcode ad-hoc signs the Simulator app with its normal simulated entitlements; this uses no personal certificate or provisioning profile. It first links a Debug Simulator app against the ordinary Release core and verifies that VM mode refuses to start before creating normal or VM state. It saves `wrong-core.json` and `wrong-core.png`, then builds the special Debug core and its Simulator app.

The test boots a disposable clone of the prepared guest, creates fresh local control/DERP and mock relay state, and connects the guest through restricted SSH forwarding. It confirms only the expected **VM Simulator** candidate through the ordinary mutual-TLS pairing flow and sends an encrypted mock notification. Finally it revokes that peer with the ordinary daemon command and requires the next push to fail without changing the saved encrypted payload.

The expected completion line begins `PASS: guest/Simulator pairing and encrypted mock push` and prints its private run directory. Inspect `runs/<run-id>/result.json` for the exact revision and result, and the adjacent build, wrong-core, guest posture, pairing, push and ordinary peer-removal evidence when diagnosing a failure. A saved encrypted payload is mock acceptance evidence; it does not prove a displayed notification or notification extension decryption.

The foreground command stops its owned guest, dedicated Simulator and test services when it succeeds, fails or receives Ctrl-C. It leaves the result and disposable state for inspection, rather than leaving an interactive app session running. Use the prepared guest desktop during its explicit desktop acceptance checks; a completed `vm-test` is not a persistent desktop-control session.

## Stop, reset and rerun

```sh
just vm-stop
```

This stops recorded test resources without deleting their state. For host processes, it checks the recorded process start and command before stopping their owned process groups, and refuses a reused PID. It must not stop an installed daemon, unrelated VM, shared Simulator or another process that happens to occupy a test port.

For another fresh integration run:

```sh
just vm-reset
just vm-test
```

Reset stops the owned resources, deletes `collie-374-test`, erases the dedicated Simulator and clears the previous run marker. It retains the pinned base, `collie-374-desktop`, tool cache and private logs. The next test clones the prepared baseline with its guest Driver grants and creates fresh tailnet authentication, relay identity, SSH keys, Simulator state and pairing. Inspect each run's own result and cleanup evidence; reset retains earlier private logs for diagnosing failed attempts.

## Remove owned resources

```sh
just vm-clean
```

Clean removes the recorded disposable guest, dedicated Simulator, prepared desktop, pinned baseline and task-owned OCI image. It retains private logs and downloaded tools at the exact directory printed by the command. Once cleanup has succeeded and the evidence is no longer needed, remove that printed task directory to delete the remaining tool archives and logs. Do not substitute a global Tart cache, shared Simulator directory or installed Collie data directory.

## Troubleshooting

| Failure | Action |
|---|---|
| Missing iOS 27 runtime | Install that Simulator runtime through Xcode, then retry setup |
| Insufficient free space or archive checksum mismatch | Free the required space or investigate the download; do not change pins to make the check pass |
| Guest permission check fails or remains stale after a toggle | In the guest Terminal, run `/Applications/CuaDriver.app/Contents/MacOS/cua-driver permissions grant`, complete the visible guest prompts including any direct-capture Allow, then retry `just vm-desktop-ready` on the host |
| Test ports `127.0.0.1:8788` through `8791` occupied | Identify their owner and stop it yourself if appropriate; the harness refuses to evict another process |
| Previous run marker present | Run `just vm-reset` before another test |
| Recorded Simulator missing or ownership does not match | Inspect the recorded state; do not adopt or delete an unrelated resource to bypass the check |
| Guest SSH startup timeout | Inspect the private run's `ssh-ready-*.log` files and guest log, preserve the failed attempt, then stop/reset only owned resources before rerunning |
| Build, guest boot, pairing or mock push timeout | Inspect the printed private run directory, stop owned resources, and preserve the failure evidence before resetting |

## Acceptance limits

The pinned image is a CI base, not a pristine consumer Mac: Gatekeeper assessments and System Integrity Protection are disabled in the guest. Debug integration or loading signed downloads in this image cannot establish clean-consumer Gatekeeper acceptance. Do not change those security settings on the host to make a test pass. Actual immutable release assets need the separate [desktop release acceptance checks](desktop-releases.md).

The local relay uses a fresh software identity and mock delivery. Genuine Apple App Attest, Secure Enclave trust, production APNs delivery and locked/background/pre-first-unlock behavior require a physical iPhone with separate evidence. Simulator success does not authorize public relay activation. See [Simulator relay testing](simulator.md) for its distinct modes and [notifications](notifications.md) for device acceptance. Linux x86_64 and Omarchy acceptance remain separate requirements.
