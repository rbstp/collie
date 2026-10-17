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

VM disks, downloaded tools, ownership records and private logs live at `<repo-parent>/.codex/vms/collie-374`, shared by this repository's worktrees. The dedicated Simulator holds its own app and Keychain state, and build outputs remain under `target`. Only one harness command may run at a time. The dedicated host Simulator is named **Collie VM test**; its exact identifier and VM ownership are recorded in `owned.json`. Treat this directory as private: it contains disposable authentication material, pairing state and test logs. Do not commit or share it.

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

SSH uses the pinned Tart guest-agent stream to reach the existing guest listener. The client still verifies the fresh pinned guest host key and uses only the task client key. The three reverse forwards listen on guest loopback only. This avoids a direct host-to-guest LAN connection without changing host VPN or routing settings; it does not establish real corporate VPN coexistence.

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

## Check the Release sample workspace

This is separate from the Debug integration above. Use the dedicated Simulator, ordinary Release core and a Release app with no demo arguments. The iOS 27.2 Simulator acceptance passed for Welcome entry, simulated denial, terminal Escape, a prompt response, Machines, actual reset of changed sample data, exit and re-entry. Ordinary preferences remained unchanged. This does not establish physical iPhone acceptance or preservation of a real signed-in account or pairing.

From the checkout, reset the harness-owned test resources, retaining the prepared baseline, and build the app:

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

This uses Simulator ad-hoc signing, without a personal certificate or provisioning profile, and seeds only benign ordinary preferences in the freshly erased dedicated Simulator. Do not use the Debug VM core, demo flags or a relay override. In Simulator, select **Collie VM test** and check:

1. On **Welcome**, choose **Explore sample workspace** without signing in. Keep the **Sample workspace / Simulated actions only** banner visible.
2. In Agents, choose the list layout and **add approval tests**. Deny its pending approval and require the visible simulated denial. In Terminal, open Keys and send Escape, then return to Chat and require the simulated terminal-key result. Send a sample prompt and require its visible simulated response. Inspect Machines.
3. In sample Settings, choose **Reset sample workspace**. Reopen the same agent: it must be blocked again with Approve and Deny available, and the prior denial and prompt response must be gone. The banner alone does not prove reset.
4. Exit to Welcome, then enter the sample again. Check that the ordinary onboarding and existing local preferences have not changed. Repeat `shasum -a 256 "$collie_sample_state/prefs.json"` and require the original digest. Confirm `machines.json` and `tsnet/tailscaled.state` remain absent from that state directory. Do not copy a real account or pairing into this test.

Keep screenshots and the exact source, Release app/core hashes and signing results with the acceptance record. The existing `SampleWorkspaceTests` covers the representative actions and exit; its reset assertion checks only the banner, so the restored-state check above is also needed. Stop and erase only this dedicated Simulator afterward:

```sh
xcrun simctl shutdown "$collie_sample_sim"
xcrun simctl erase "$collie_sample_sim"
```

## Check published desktop downloads in a separate guest

`vm-test` uses host-only networking and Debug artifacts. Published downloads require a separately named clone with default shared NAT for GitHub HTTPS and Apple's online notarization lookup. NAT permits general outbound access, not an Apple-only allowlist. Use the stopped, consented `collie-374-desktop` baseline; never install into it or copy host credentials, home directories or existing tailnet state into the clone.

Choose one approved published tag, replace the tag placeholder below, and record its exact source from `source.json`, selected asset hashes, clone name and baseline identity in a separate private acceptance record. Confirm the name is absent in `tart list` before cloning. From the checkout:

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

Leave `--net-host` off this run. Add no bridge, SSH listener, shares, host proxy or host network/security changes. Use the guest desktop and its existing signed Driver grants. Record the actual guest OS, empty installed-product state, Driver socket/permissions and CI security posture before testing.

If guest HTTPS times out while routing and DNS work, record the current guest `en0` MTU first. On the tested host VPN path, guest MTU 1500 failed, 1280 passed the same HTTPS probes, and restoring 1500 reproduced the failure. The scoped workaround is temporary and guest-only. In the guest Terminal:

```sh
collie_guest_original_mtu="$(/sbin/ifconfig en0 | awk 'NR == 1 {print $NF}')"
printf 'Original guest MTU: %s\n' "$collie_guest_original_mtu"
sudo -n /sbin/ifconfig en0 mtu 1280
```

Keep this guest Terminal open and save the displayed original MTU in the acceptance record. Do not treat this as a certificate fix or change host interfaces, VPN or DNS. Restore that actual original value before deleting the clone; it was 1500 in the verified experiment.

Use guest Safari to download the selected macOS assets, `source.json` and `SHA256SUMS` from that exact release. Record actual quarantine attributes before opening anything, verify each selected manifest entry uniquely and compare the source identity. Follow the existing [download installation and trust checks](desktop-releases.md): retain strict Developer ID/team/identifier checks, the standalone daemon's explicit online notarization check, and app/DMG policy checks. The raw daemon check is `codesign --verify --strict -R=notarized --check-notarization ./collied`. Never re-sign, remove quarantine or skip a failed check. If browser downloads fail but verified HTTPS works, a CLI transfer can support bounded install tests; browser/quarantine acceptance remains unproven.

The authentic v0.0.152 downloads passed guest checksum, Safari quarantine and explicit signature/notarization checks, but its installer exited 141 before copying the executable because a metadata pipeline failed under `pipefail`. Published install, service and app acceptance did not pass. An unreleased installer candidate against that unchanged signed binary is separate evidence; do not present it as a fixed published download.

Run the packaged installer with `--replace` only inside this disposable guest and stop at its first failure. Continue only after it succeeds: record the installed binary hash/version/signature, then check setup prerequisites and cancel if it reaches login without signing in. Setup logs in before installing its service. Separately check service install/start/stop/uninstall and CollieBar's initial Off state, installed-service discovery and Quit. A registered service or Starting/Off menu is not a healthy authenticated daemon. Real Running, full setup, pairing and agent control need legitimate dedicated test-tailnet authorization and guest prerequisites; never use Debug control/auth bypasses for Release. Do not send APNs or activate a publisher relay in this lane.

Quit CollieBar, disable any guest login item enabled during the test, uninstall its guest service and detach any mounted DMG. If you applied the MTU workaround, use the same guest Terminal to restore and verify the captured original value:

```sh
sudo -n /sbin/ifconfig en0 mtu "$collie_guest_original_mtu"
/sbin/ifconfig en0
```

Then, from a second host Terminal with the same variables, stop and delete only the separately recorded clone:

```sh
"$collie_tart" stop "$collie_release_vm"
"$collie_tart" delete "$collie_release_vm"
"$collie_tart" list
```

Verify its owned process has exited, its name is absent, and the stopped prepared baseline and harness ownership record are unchanged. Keep private download, trust, screenshot, service, MTU-restoration and cleanup evidence. Do not run `vm-clean` or remove the harness task directory while a separate acceptance clone exists. This CI base has SIP and Gatekeeper assessments disabled, so even successful explicit trust/install checks cannot establish pristine-consumer Gatekeeper acceptance.

## Acceptance limits

The pinned image is a CI base, not a pristine consumer Mac: Gatekeeper assessments and System Integrity Protection are disabled in the guest. Debug integration or loading signed downloads in this image cannot establish clean-consumer Gatekeeper acceptance. Do not change those security settings on the host to make a test pass. Published versioned release assets need the separate [desktop release acceptance checks](desktop-releases.md).

The local relay uses a fresh software identity and mock delivery. Genuine Apple App Attest, Secure Enclave trust, production APNs delivery and locked/background/pre-first-unlock behavior require a physical iPhone with separate evidence. Simulator success does not authorize public relay activation. See [Simulator relay testing](simulator.md) for its distinct modes and [notifications](notifications.md) for device acceptance. Linux x86_64 and Omarchy acceptance remain separate requirements.
