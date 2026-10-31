# macOS VM and host Simulator testing

This harness runs the test daemon in a disposable macOS guest and the iOS Simulator on the Apple silicon host, with its own local tailnet control, mock relay, identities and pairing. It needs no Tailscale or Cloudflare account and no Apple signing credentials, and it leaves the installed owner daemon and personal direct APNs configuration alone.

Two consecutive fresh `just vm-reset` and `just vm-test` runs passed at clean revision `90f747c7f66f3f7e0c328b2702bcc0a5c767f892`, each reproving guest desktop capture, a Calculator accessibility click changing `0` to `6`, VM mode refusal with an ordinary Release core, isolated VM identities, ordinary mutual-TLS pairing, encrypted mock push, and ordinary peer removal with a rejected follow-up. The prepared guest kept its Driver grants without another consent round.

## Requirements and pinned inputs

You need the [build prerequisites](../install/requirements.html), including initialized submodules, Rust, Go, just, Xcode, XcodeGen, jq and Node with npm. The harness requires an Apple silicon macOS 27 host and an available iOS 27 Simulator runtime, and reads Go from `/opt/homebrew/opt/go/bin/go`. The exercised host is macOS 27.2; other host and runtime combinations have no acceptance result in this lane.

Reserve at least 80 GiB free for VM setup, plus space for Rust and Xcode build outputs and the existing Xcode and Simulator installation. The image download is 34.13 GB and its disk metadata reports 41.05 GB. The guest gets two virtual CPUs and 4 GiB RAM, so leave memory for Xcode and Simulator on the host.

| Input | Pin |
|---|---|
| Tart | `2.41.1`, archive SHA-256 `e680a734febccfce7d4e910c1f7f13fc3b00f19a181c945e2d746de3214c32c2` |
| macOS base | `ghcr.io/cirruslabs/macos-golden-gate-base@sha256:5302a9ea6cd68c059d00fa253fe75c8ebdf4e0dc8c45d4e136ecc6e7a4c02b70` |
| Guest CuaDriver | `0.36.0`, archive SHA-256 `4a0e229922a1ac6d6f59152e0653541920951afa89a3007e4ff5ffc024eae7a8` |

The guest boots as macOS 27.0, build `26A428`. The harness downloads Tart into its own tool directory, verifies its signature and Gatekeeper assessment, and uses a separate `TART_HOME`. It installs the signed CuaDriver app only inside the guest and disables its telemetry. Driver control uses a guest `admin`-owned UNIX socket with mode `0600` and no TCP listener. The procedure needs no global Tart installation and no host Accessibility or screen-recording grant.

VM disks, downloaded tools, ownership records and private logs live in `<repo-parent>/.codex/vms/collie-374`, shared by this repository's worktrees; build outputs stay under `target`. The dedicated host Simulator, **Collie VM test**, holds its own app and Keychain state, and `owned.json` records its exact identifier and VM ownership. Only one harness command can run at a time. Keep the directory private, uncommitted and unshared: it holds disposable authentication material, pairing state and test logs.

## Set up the guest desktop once

From the repository checkout:

```sh
just setup
just vm-setup
```

The first setup downloads the pinned base, creates the dedicated iPhone 18 Pro Simulator, opens `collie-374-test` and installs CuaDriver in the guest. Downloads and builds need network access. It never takes ownership of unrelated VMs, Simulators, installed daemon state, Keychain items or real pairings.

Log in to the guest as `admin` with password `admin`, the public defaults of this CI image. In the guest, grant CuaDriver **Accessibility** and **Screen & System Audio Recording**, accept its relaunch, and complete any System Events or direct capture prompt. Then, on the host, run:

```sh
just vm-desktop-ready
```

This checks that both grants are attributed to the Driver daemon, clicks Calculator through its accessibility tree to prove a visible `0` to `6` change, lists guest apps and captures PNG screenshots, including Calculator before and after. It saves this private evidence under `desktop-proof`, stops the guest and creates `collie-374-desktop`, a prepared baseline that keeps the guest permission decisions without run-specific relay, SSH or pairing credentials. On success it prints `Guest desktop grants and capture verified; prepared baseline collie-374-desktop` and the evidence directory. The check does not claim that every app can be controlled.

## Run the integration test

```sh
just vm-test
```

Keep this command in the foreground. Xcode ad-hoc signs the Simulator app with its normal simulated entitlements, without a personal certificate or provisioning profile. The test first links a Debug Simulator app against the ordinary Release core and verifies that VM mode refuses to start before creating normal or VM state, saving `wrong-core.json` and `wrong-core.png`. It then builds the special Debug core and its Simulator app.

The test also boots a disposable clone of the prepared guest, creates fresh local control, DERP and mock relay state, and connects the guest through restricted SSH forwarding. It confirms only the expected **VM Simulator** candidate through the ordinary mutual-TLS pairing flow and sends an encrypted mock notification. Finally it revokes that peer with the ordinary daemon command and requires the next push to fail without changing the saved encrypted payload.

SSH reaches the guest listener through the pinned Tart guest-agent stream. The client still verifies the fresh pinned guest host key and uses only the task client key, and the three reverse forwards listen on guest loopback only. This avoids a direct host-to-guest LAN connection without changing host VPN or routing settings; it does not establish real corporate VPN coexistence.

On success the command prints a line beginning `PASS: guest/Simulator pairing and encrypted mock push` and its private run directory. `runs/<run-id>/result.json` records the exact revision and result; to diagnose a failure, inspect the adjacent build, wrong-core, guest posture, pairing, push and peer-removal evidence. The saved encrypted payload shows mock acceptance only, not a displayed notification or notification extension decryption.

Whether the command succeeds, fails or receives Ctrl-C, it stops its owned guest, dedicated Simulator and test services and leaves the result and disposable state for inspection. A completed `vm-test` is not a persistent desktop-control session; for desktop control, use the prepared guest desktop during its explicit desktop acceptance checks.

## Stop, reset and rerun

```sh
just vm-stop
```

This stops recorded test resources without deleting their state. Before stopping an owned host process group, it checks the recorded process start time and command, so it refuses a reused PID. It must not stop an installed daemon, unrelated VM, shared Simulator or another process that happens to hold a test port.

For another fresh integration run:

```sh
just vm-reset
just vm-test
```

Reset stops the owned resources, deletes `collie-374-test`, erases the dedicated Simulator and clears the previous run marker. It keeps the pinned base, `collie-374-desktop`, the tool cache and earlier private logs for diagnosing failed attempts, so inspect each run's own result and cleanup evidence. The next test clones the prepared baseline with its guest Driver grants and creates fresh tailnet authentication, relay identity, SSH keys, Simulator state and pairing.

## Remove owned resources

```sh
just vm-clean
```

Clean removes the recorded disposable guest, dedicated Simulator, prepared desktop, pinned baseline and task-owned OCI image. It keeps private logs and downloaded tools in the directory it prints. Once cleanup succeeds and you no longer need the evidence, remove that printed directory to delete the remaining tool archives and logs. Do not substitute a global Tart cache, shared Simulator directory or installed Collie data directory.

## Troubleshooting

| Failure | Action |
|---|---|
| Missing iOS 27 runtime | Install that Simulator runtime through Xcode, then retry setup |
| Insufficient free space or archive checksum mismatch | Free the required space or investigate the download; do not change pins to make the check pass |
| Guest permission check fails or stays stale after a toggle | In the guest Terminal, run `/Applications/CuaDriver.app/Contents/MacOS/cua-driver permissions grant`, complete the visible guest prompts including any direct-capture Allow, then retry `just vm-desktop-ready` on the host |
| Test ports `127.0.0.1:8788` through `8791` occupied | Identify their owner and stop it yourself if appropriate; the harness refuses to evict another process |
| Previous run marker present | Run `just vm-reset` before another test |
| Recorded Simulator missing or ownership does not match | Inspect the recorded state; do not adopt or delete an unrelated resource to bypass the check |
| Guest SSH startup timeout | Inspect the private run's `ssh-ready-*.log` files and guest log, keep the failed attempt, then stop or reset only owned resources before rerunning |
| Build, guest boot, pairing or mock push timeout | Inspect the printed private run directory, stop owned resources, and keep the failure evidence before resetting |

## Check the Release sample workspace

This check is separate from the Debug integration above. It uses the dedicated Simulator, the ordinary Release core and a Release app with no demo arguments. The iOS 27.2 Simulator acceptance passed for Welcome entry, simulated denial, terminal Escape, a prompt response, Machines, actual reset of changed sample data, exit and re-entry, and ordinary preferences stayed unchanged. It does not establish physical iPhone acceptance or preservation of a real signed-in account or pairing.

From the checkout, reset the harness-owned test resources (the prepared baseline stays) and build the app:

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

The build is ad-hoc signed, and the commands seed only benign ordinary preferences in the freshly erased dedicated Simulator. Do not use the Debug VM core, demo flags or a relay override. In Simulator, select **Collie VM test** and check:

1. On **Welcome**, choose **Explore sample workspace** without signing in. Keep the **Sample workspace / Simulated actions only** banner visible.
2. In Agents, choose the list layout and **add approval tests**. Deny its pending approval and require the visible simulated denial. In Terminal, open Keys and send Escape, then return to Chat and require the simulated terminal-key result. Send a sample prompt and require its visible simulated response. Inspect Machines.
3. In sample Settings, choose **Reset sample workspace**. Reopen the same agent: it must be blocked again with Approve and Deny available, and the earlier denial and prompt response must be gone. The banner alone does not prove reset.
4. Exit to Welcome, then enter the sample again. Check that the ordinary onboarding and existing local preferences have not changed: repeat `shasum -a 256 "$collie_sample_state/prefs.json"` and require the original digest. Confirm that `machines.json` and `tsnet/tailscaled.state` are still absent from that state directory. Do not copy a real account or pairing into this test.

Keep screenshots, the exact source, Release app and core hashes and signing results with the acceptance record. The existing `SampleWorkspaceTests` covers the representative actions and exit, but its reset assertion checks only the banner, so the restored-state check above is still needed. Afterwards, stop and erase only this dedicated Simulator:

```sh
xcrun simctl shutdown "$collie_sample_sim"
xcrun simctl erase "$collie_sample_sim"
```

## Check published desktop downloads in a separate guest

`vm-test` uses host-only networking and Debug artifacts. Published downloads need a separately named clone with default shared NAT, for GitHub HTTPS and Apple's online notarization lookup. NAT permits general outbound access, not an Apple-only allowlist. Clone the stopped, consented `collie-374-desktop` baseline; never install into it, and never copy host credentials, home directories or existing tailnet state into the clone.

Choose one approved published tag and replace the tag placeholder below. In a separate private acceptance record, note its exact source from `source.json`, the selected asset hashes, the clone name and the baseline identity. Confirm that the name is absent from `tart list` before cloning. From the checkout:

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

Leave `--net-host` off this run, and add no bridge, SSH listener, shares, host proxy or host network or security changes. Use the guest desktop and its existing signed Driver grants. Before testing, record the actual guest OS, the empty installed-product state, the Driver socket and permissions, and the CI security posture.

If guest HTTPS times out while routing and DNS work, first record the guest's current `en0` MTU. On the tested host VPN path, guest MTU 1500 failed, 1280 passed the same HTTPS probes, and restoring 1500 reproduced the failure. The workaround is temporary and guest-only. In the guest Terminal:

```sh
collie_guest_original_mtu="$(/sbin/ifconfig en0 | awk 'NR == 1 {print $NF}')"
printf 'Original guest MTU: %s\n' "$collie_guest_original_mtu"
sudo -n /sbin/ifconfig en0 mtu 1280
```

Keep this guest Terminal open and save the displayed original MTU in the acceptance record. This is not a certificate fix; do not change host interfaces, VPN or DNS. Restore the original value before deleting the clone (it was 1500 in the verified experiment).

Use guest Safari to download the selected macOS assets, `source.json` and `SHA256SUMS` from that exact release. Record the quarantine attributes before opening anything, verify each selected manifest entry uniquely and compare the source identity. Follow the [download installation and trust checks](desktop-releases.md#install-a-daemon-download): keep the strict Developer ID, team and identifier checks, the standalone daemon's explicit online notarization check, and the app and DMG policy checks. The raw daemon check is `codesign --verify --strict -R=notarized --check-notarization ./collied`. Never re-sign, remove quarantine or skip a failed check. If browser downloads fail but verified HTTPS works, a CLI transfer can support bounded install tests, but browser and quarantine acceptance stays unproven.

The authentic v0.0.152 downloads passed guest checksum, Safari quarantine and explicit signature and notarization checks, but the installer exited 141 before copying the executable because a metadata pipeline failed under `pipefail`. Published install, service and app acceptance did not pass. An unreleased installer candidate tested against that unchanged signed binary is separate evidence; do not present it as a fixed published download.

Run the packaged installer with `--replace` only inside this disposable guest, and stop at its first failure. Only after it succeeds, record the installed binary's hash, version and signature, then check setup prerequisites and cancel if setup reaches login without signing in (setup logs in before installing its service). Separately check service install, start, stop and uninstall, and CollieBar's initial Off state, installed-service discovery and Quit. A registered service or a Starting or Off menu is not a healthy authenticated daemon. Real Running, full setup, pairing and agent control need legitimate dedicated test-tailnet authorization and guest prerequisites; never use Debug control or auth bypasses for Release. Do not send APNs or activate a publisher relay in this lane.

Quit CollieBar, disable any guest login item enabled during the test, uninstall its guest service and detach any mounted DMG. If you applied the MTU workaround, restore and verify the captured value in the same guest Terminal:

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

Verify that its owned process has exited, its name is gone, and the stopped prepared baseline and harness ownership record are unchanged. Keep the private download, trust, screenshot, service, MTU-restoration and cleanup evidence. Do not run `vm-clean` or remove the harness task directory while a separate acceptance clone exists.

## Acceptance limits

The pinned image is a CI base, not a pristine consumer Mac: the guest has Gatekeeper assessments and System Integrity Protection disabled, so neither Debug integration nor explicit trust and install checks of signed downloads in this image can establish clean-consumer Gatekeeper acceptance. Do not change those security settings on the host to make a test pass. Published release assets need the separate [desktop release acceptance checks](desktop-releases.md#validation-before-public-rollout).

The local relay uses a fresh software identity and mock delivery. Genuine Apple App Attest, Secure Enclave trust, production APNs delivery and locked, background or pre-first-unlock behavior need separate evidence from a physical iPhone. See [Simulator relay testing](simulator.md) for its modes and [notifications](notifications.md) for device acceptance. Linux x86_64 and Omarchy acceptance remain separate requirements.
