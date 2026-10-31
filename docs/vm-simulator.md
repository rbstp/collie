# macOS VM and host Simulator testing

This harness runs the test daemon in a disposable macOS guest and the iOS Simulator on the Apple silicon host, with its own local tailnet control, mock relay, identities and pairing ([design](architecture.md#isolated-vm-and-simulator-tailnet)). It needs no Tailscale, Cloudflare or Apple signing account, and leaves the installed daemon and personal APNs configuration alone.

Two consecutive fresh `just vm-reset` and `just vm-test` runs passed at clean revision `90f747c7f66f3f7e0c328b2702bcc0a5c767f892`, each reproving guest desktop capture, a Calculator accessibility click changing `0` to `6`, VM mode refusal with an ordinary Release core, isolated VM identities, ordinary mutual-TLS pairing, encrypted mock push, and ordinary peer removal with a rejected follow-up. The prepared guest kept its Driver grants without another consent round.

## Requirements and pinned inputs

You need the [build prerequisites](../install/requirements.html), Node with npm, initialized submodules, an Apple silicon macOS 27 host (only 27.2 is tested) and an iOS 27 Simulator runtime. The harness reads Go from `/opt/homebrew/opt/go/bin/go`.

Reserve at least 80 GiB free, plus room for build outputs: the image download is 34.13 GB, and its disk metadata reports 41.05 GB. The guest takes two virtual CPUs and 4 GiB RAM.

| Input | Pin |
|---|---|
| Tart | `2.41.1`, archive SHA-256 `e680a734febccfce7d4e910c1f7f13fc3b00f19a181c945e2d746de3214c32c2` |
| macOS base | `ghcr.io/cirruslabs/macos-golden-gate-base@sha256:5302a9ea6cd68c059d00fa253fe75c8ebdf4e0dc8c45d4e136ecc6e7a4c02b70` |
| Guest CuaDriver | `0.36.0`, archive SHA-256 `4a0e229922a1ac6d6f59152e0653541920951afa89a3007e4ff5ffc024eae7a8` |

The guest boots macOS 27.0, build `26A428`. The harness keeps its verified Tart in its own tool directory and `TART_HOME`. CuaDriver runs only in the guest, with telemetry off, behind a guest `admin`-owned `0600` UNIX socket and no TCP listener. The host needs no Accessibility or screen-recording grant.

Disks, tools, ownership records and logs live in `<repo-parent>/.codex/vms/collie-374`, shared by this repository's worktrees. Keep that directory private and uncommitted: it holds disposable credentials, pairing state and logs. The dedicated Simulator, **Collie VM test**, has its own app and Keychain state, and `owned.json` records it. One harness command runs at a time.

## Set up the guest desktop once

```sh
just setup
just vm-setup
```

The first setup downloads the pinned base, creates the dedicated iPhone 18 Pro Simulator, boots `collie-374-test` and installs CuaDriver in it. It never takes over unrelated VMs, Simulators, installed daemon state, Keychain items or real pairings.

Log in to the guest as `admin` with password `admin`, the CI image's public defaults. Grant CuaDriver **Accessibility** and **Screen & System Audio Recording**, accept its relaunch and complete any System Events or direct capture prompt. Then, on the host:

```sh
just vm-desktop-ready
```

This verifies both Driver grants, clicks Calculator from `0` to `6` through its accessibility tree and saves screenshots in `desktop-proof`. It then stops the guest and saves it as `collie-374-desktop`, a baseline with the permission decisions but no run-specific credentials.

## Run the integration test

```sh
just vm-test
```

Keep it in the foreground. It:

1. Builds a Debug app on the ordinary Release core and checks that VM mode refuses to start before creating normal or VM state (`wrong-core.json`, `wrong-core.png`), then builds the special Debug core and app.
2. Boots a disposable clone of the prepared guest with fresh local control, DERP and mock relay state, reached over restricted SSH (pinned guest host key, task key only, guest-loopback forwards) through Tart's guest agent rather than the LAN. It changes no host VPN or routing setting and proves nothing about a real corporate VPN.
3. Confirms only the **VM Simulator** candidate at the ordinary mutual-TLS pairing prompt and sends an encrypted mock notification.
4. Revokes that peer with the ordinary daemon command and requires the next push to fail without changing the saved payload.

Success prints `PASS: guest/Simulator pairing and encrypted mock push` and the private run directory; its `result.json` records the revision and result beside each step's evidence. The saved payload proves mock acceptance only, not display or decryption.

On success, failure or Ctrl-C, the command stops its guest, Simulator and test services and keeps the result and state.

## Stop, reset and rerun

```sh
just vm-stop
```

This stops recorded test resources and keeps their state. It matches each owned process group's recorded start time and command, so it never stops a reused PID, an installed daemon, an unrelated VM, a shared Simulator or another process on a test port.

For another fresh integration run:

```sh
just vm-reset
just vm-test
```

Reset stops owned resources, deletes `collie-374-test`, erases the dedicated Simulator and clears the run marker, so the next test starts from the prepared baseline with fresh credentials, keys, Simulator state and pairing. It keeps the pinned base, `collie-374-desktop`, the tool cache and earlier logs.

## Remove owned resources

```sh
just vm-clean
```

Clean removes the recorded guest, dedicated Simulator, prepared desktop, pinned baseline and task-owned OCI image, and prints the directory that keeps logs and tools. Once cleanup succeeds and you no longer need the evidence, delete that exact directory; never substitute a global Tart cache, shared Simulator directory or installed Collie data directory.

## Troubleshooting

| Failure | Action |
|---|---|
| Too little space, or checksum mismatch | Free space or investigate the download; never change pins to pass |
| Guest permission check fails or stays stale | In the guest Terminal, run `/Applications/CuaDriver.app/Contents/MacOS/cua-driver permissions grant`, complete its prompts, then retry `just vm-desktop-ready` on the host |
| Recorded Simulator missing or not owned | Inspect the recorded state; never adopt or delete an unrelated resource |
| Timeout in build, boot, SSH, pairing or push | Keep and read the printed run directory (for SSH, `ssh-ready-*.log` and the guest log), then stop or reset only owned resources |

## Check the Release sample workspace

This check runs a Release app on the ordinary Release core, with no demo arguments, on the dedicated Simulator. It passed on the iOS 27.2 Simulator for Welcome entry, simulated denial, terminal Escape, a prompt response, Machines, actual reset of changed sample data, exit and re-entry, with ordinary preferences unchanged. It proves neither physical iPhone acceptance nor that a real signed-in account or pairing survives.

From the checkout, reset the harness and build the app:

```sh
just vm-reset
export PATH="/opt/homebrew/opt/go/bin:$PATH"
just ios-framework sim release
just ios-project
collie_vm_root="$(dirname "$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")")/.codex/vms/collie-374"
collie_sample_sim="$(jq -er '.simulator' "$collie_vm_root/owned.json")"
xcodebuild -project Collie/Collie.xcodeproj -scheme Collie -configuration Release \
  -derivedDataPath target/ios/ReleaseSampleCheck \
  -destination "platform=iOS Simulator,id=$collie_sample_sim" \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=YES \
  CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Automatic build
xcrun simctl boot "$collie_sample_sim"
xcrun simctl bootstatus "$collie_sample_sim" -b
xcrun simctl install "$collie_sample_sim" target/ios/ReleaseSampleCheck/Build/Products/Release-iphonesimulator/Collie.app
collie_sample_data="$(xcrun simctl get_app_container "$collie_sample_sim" dev.rbstp.collie data)"
collie_sample_state="$collie_sample_data/Library/Application Support/collie"
mkdir -p "$collie_sample_state"
chmod 700 "$collie_sample_state"
printf '%s\n' '{"accent":"orange","agentsLayout":"grid","defaultSessionView":"chat","wrapLines":false,"historyLines":500}' > "$collie_sample_state/prefs.json"
shasum -a 256 "$collie_sample_state/prefs.json"
xcrun simctl launch "$collie_sample_sim" dev.rbstp.collie
```

Use no Debug VM core, demo flags or relay override. In Simulator, select **Collie VM test** and check:

1. On **Welcome**, choose **Explore sample workspace** without signing in. The **Sample workspace / Simulated actions only** banner must stay visible.
2. In Agents, choose the list layout and open **add approval tests**. Deny its approval and require the simulated denial. In Terminal, send Escape from Keys, then require the simulated terminal-key result in Chat. Send a sample prompt and require its simulated response. Inspect Machines.
3. In sample Settings, choose **Reset sample workspace**. Reopen the agent: it must be blocked again with Approve and Deny, without the earlier denial and response. The banner alone does not prove reset.
4. Exit to Welcome and re-enter the sample. Onboarding must be unchanged, `shasum -a 256 "$collie_sample_state/prefs.json"` must print the original digest, and `machines.json` and `tsnet/tailscaled.state` must still be absent from that directory. Never copy a real account or pairing into this test.

Record screenshots, the exact source, the Release app and core hashes and the signing results. `SampleWorkspaceTests` checks only the banner after reset, so step 3 stays manual. Then stop and erase only this Simulator:

```sh
xcrun simctl shutdown "$collie_sample_sim"
xcrun simctl erase "$collie_sample_sim"
```

## Check published desktop downloads in a separate guest

Published downloads need a separately named clone of the stopped, consented `collie-374-desktop` baseline with default shared NAT for GitHub and Apple's online notarization lookup (NAT allows any outbound host). Never install into the baseline or copy host credentials, home directories or tailnet state into the clone.

Replace `vMAJOR.MINOR.PATCH` below with one approved published tag. In a private acceptance record, note its source from `source.json`, the selected asset hashes, the clone name and the baseline identity. Confirm that `tart list` lacks the clone name, then, from the checkout:

```sh
collie_vm_root="$(dirname "$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")")/.codex/vms/collie-374"
export TART_HOME="$collie_vm_root/tart"
export TART_NO_AUTO_PRUNE=1
collie_tart="$collie_vm_root/tools/tart.app/Contents/MacOS/tart"
collie_release_tag='vMAJOR.MINOR.PATCH'
collie_release_vm="collie-release-acceptance-$collie_release_tag"
"$collie_tart" list
"$collie_tart" clone collie-374-desktop "$collie_release_vm"
"$collie_tart" set "$collie_release_vm" --cpu 2 --memory 4096
"$collie_tart" run "$collie_release_vm" --no-audio --no-usb-accessories --no-clipboard
```

Leave `--net-host` off, and add no bridge, SSH listener, shares, host proxy, or host network or security change. Before testing, record the guest OS, the empty installed-product state, the Driver socket and permissions, and the CI security posture.

If guest HTTPS times out while routing and DNS work, record the guest's `en0` MTU first. On the tested host VPN path, guest MTU 1500 failed, 1280 passed the same HTTPS probes, and restoring 1500 reproduced the failure. In the guest Terminal, apply the temporary, guest-only workaround:

```sh
collie_guest_original_mtu="$(/sbin/ifconfig en0 | awk 'NR == 1 {print $NF}')"
printf 'Original guest MTU: %s\n' "$collie_guest_original_mtu"
sudo -n /sbin/ifconfig en0 mtu 1280
```

Keep this Terminal open and record the printed original MTU (1500 in the verified experiment). This is not a certificate fix; never change host interfaces, VPN or DNS.

With guest Safari, download that release's selected macOS assets, `source.json` and `SHA256SUMS`. Record quarantine attributes before opening anything, verify each selected manifest entry uniquely and compare the source identity. Run every [download trust check](desktop-releases.md#install-a-daemon-download), including the daemon's online notarization check (`codesign --verify --strict -R=notarized --check-notarization ./collied`) and the app and DMG policy checks; never re-sign, remove quarantine or skip a failed check. If only browser downloads fail, a verified CLI transfer can support bounded install tests, but not browser or quarantine acceptance.

The authentic v0.0.152 downloads passed guest checksum, Safari quarantine and explicit signature and notarization checks, but the installer exited 141 before copying the executable: a metadata pipeline failed under `pipefail`. Published install, service and app acceptance did not pass. An unreleased installer candidate tested against that unchanged signed binary is separate evidence, not a fixed published download.

Run the packaged installer with `--replace` only in this guest and stop at its first failure. Once it succeeds, record the installed binary's hash, version and signature, run setup to check prerequisites, and cancel at the login step (setup logs in before installing the service). Check service install, start, stop and uninstall separately, and CollieBar's initial Off state, service discovery and Quit; a registered service or a Starting or Off menu is not a healthy authenticated daemon. Running, full setup, pairing and agent control need a legitimate dedicated test tailnet and guest prerequisites. Never use Debug control or auth bypasses for Release, send APNs or activate a publisher relay here.

To clean up, quit CollieBar, disable any guest login item the test enabled, uninstall the guest service and detach any mounted DMG. If you lowered the MTU, restore and verify it in the same guest Terminal:

```sh
sudo -n /sbin/ifconfig en0 mtu "$collie_guest_original_mtu"
/sbin/ifconfig en0
```

Then, from a second host Terminal with the same variables, stop and delete only the recorded clone:

```sh
"$collie_tart" stop "$collie_release_vm"
"$collie_tart" delete "$collie_release_vm"
"$collie_tart" list
```

Verify that its process exited, its name is gone, and the baseline and harness ownership record are unchanged. Keep the private evidence, including MTU restoration and cleanup. Never run `vm-clean` or remove the harness directory while an acceptance clone exists.

## Acceptance limits

The pinned image is a CI base with Gatekeeper assessments and System Integrity Protection disabled, so no test in it establishes clean-consumer Gatekeeper acceptance; never change those settings on the host to pass a test. Published assets also need the [desktop release acceptance checks](desktop-releases.md#validation-before-public-rollout).

Genuine App Attest, Secure Enclave trust, production APNs delivery and locked, background or pre-first-unlock behavior need a physical iPhone ([Simulator relay testing](simulator.md), [notifications](notifications.md)). Linux x86_64 and Omarchy acceptance are separate.
