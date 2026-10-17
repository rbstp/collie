# Test Collie 0.1.1 on iOS and Linux

Use the published [0.1.1 release](https://github.com/rbstp/collie/releases/tag/v0.1.1) for this Linux-first test. The commands below verify the downloads, privately back up old Collie state and set up a fresh embedded Linux node with the public push relay.

The public Linux package is version 0.1.1, source b23d0257421f77b9ad4a9b69bea6a633871baa9b, protocol 26, x86_64, glibc 2.39+, systemd 255+. The phone must show TestFlight 0.1.1 (195). Keep the same phone state and Mac Direct APNs configuration; Linux uses the relay.

## Update the iPhone

1. Open TestFlight, select Collie and update to **0.1.1 (195)**. Confirm that exact version and build before pairing. Keep the installed app and its data; do not delete or reset it.
2. Sign in inside Collie to the same personal tailnet account used for the new Linux node. Keep the existing user-owned phone node. The standalone Tailscale app and an OS VPN connection are not required.
3. Allow Collie notifications, then keep the app foreground during pairing and notification registration.

Leave the Mac on Direct APNs with its existing service, configuration and corporate VPN. A legacy Mac pairing can remain unavailable until its separately coordinated identity update. This procedure does not convert the Mac to the relay.

## Download and verify, before changing old state

Run on Linux in a shell that keeps the variables for the following steps:

```sh
umask 077
collie_download_dir="$(mktemp -d "${TMPDIR:-/tmp}/collie-0.1.1.XXXXXX")"
(
set -eu
cd "$collie_download_dir"
curl --fail --location --output collied-0.1.1-linux-x86_64.tar.gz https://github.com/rbstp/collie/releases/download/v0.1.1/collied-0.1.1-linux-x86_64.tar.gz
curl --fail --location --output source.json https://github.com/rbstp/collie/releases/download/v0.1.1/source.json
printf '%s  %s\n' 5644296da020240169c4fc821665bf0405d9e6d84a947dd8a0dd643d23975288 collied-0.1.1-linux-x86_64.tar.gz d17ae23e7fdb1f421dffdf186d76d85c9636645845fc9a08d857e011ef26a131 source.json | sha256sum --check -
python3 - <<'CHECK_SOURCE'
import json
with open("source.json") as file:
    source = json.load(file)
assert source == {"tag": "v0.1.1", "source_sha": "b23d0257421f77b9ad4a9b69bea6a633871baa9b"}
CHECK_SOURCE
tar -xzf collied-0.1.1-linux-x86_64.tar.gz
cd collied-0.1.1-linux-x86_64
./collied --version
)
```

Stop on a download, checksum, source check, extraction or runtime error. Require exactly collied 0.1.1. Do not run the installer yet if existing Collie state needs the reset below.

## Inspect existing Collie before a fresh-state reset

```sh
systemctl --user cat collied.service
systemctl --user show collied.service --property=FragmentPath,DropInPaths,ExecStart,Environment
```

Keep output private. The following reset applies only to the normal Collie unit with the same XDG roots as this shell, no drop-ins, no external --config and no independently launched daemon. A missing old service is a normal fresh installation; skip its stop/uninstall, but still preserve any old Collie data before setup. A custom or running standalone daemon needs a tailored stop, not a kill or guessed path.

For an existing ordinary unit, run the old installed binary:

```sh
~/.cargo/bin/collied stop
systemctl --user is-active collied.service
```

Require inactive, and confirm no collied process remains for this user. Then back up only Collie state and its unit; run the groups individually and stop on errors:

```sh
case "${XDG_DATA_HOME:-}" in
  /*) collie_data_parent="$XDG_DATA_HOME" ;;
  *) collie_data_parent="$HOME/.local/share" ;;
esac
case "${XDG_CONFIG_HOME:-}" in
  /*) collie_config_root="$XDG_CONFIG_HOME" ;;
  *) collie_config_root="$HOME/.config" ;;
esac
collie_data_dir="$collie_data_parent/collie"
mkdir -p "$collie_data_parent"
collie_backup_dir="$(mktemp -d "$collie_data_parent/collie-before-0.1.1.XXXXXX")" || exit 1
chmod 700 "$collie_backup_dir" || exit 1
if [ -f "$collie_config_root/systemd/user/collied.service" ]; then
  cp -p "$collie_config_root/systemd/user/collied.service" "$collie_backup_dir/collied.service"
fi
```

For the existing ordinary unit, remove only that unit using the old installed binary, after its private backup:

```sh
~/.cargo/bin/collied service uninstall
```

Move the old data into the sibling private backup if it exists:

```sh
if [ -e "$collie_data_dir" ]; then
  mv "$collie_data_dir" "$collie_backup_dir/state"
fi
printf 'Private Collie backup: %s\n' "$collie_backup_dir"
```

Do not restore old config, peers, tsnet or push state into the new directory. An external old APNs key_path file remains private in place; the data move does not copy it. In personal Tailscale admin remove only the old tagged Collie node collie-omarchy; preserve native omarchy, collie-phone and all Mac nodes. Do not touch native Tailscale directories or services.

## Install the verified package and select relay

Return to the extracted verified directory if necessary:

```sh
(
set -eu
cd "$collie_download_dir/collied-0.1.1-linux-x86_64"
bash install.sh --replace
collie_installed_version="$(~/.cargo/bin/collied --version)"
test "$collie_installed_version" = "collied 0.1.1"
printf '%s\n' "$collie_installed_version"
~/.cargo/bin/collied setup
)
```

Require collied 0.1.1. Sign in to the same personal account as the phone using Collie's embedded login. Manual device approval remains enabled: approve only this exact fresh, user-owned, untagged Linux Collie node in the personal admin Machines menu, then rerun setup if it was waiting/stopped. Never use an old tagged auth key. The personal-tailnet policy is global member-to-self; Linux needs no special rule.

Answer y to push setup and y to pairing. Do not pass --direct-apns and do not import an Apple key. Fresh setup writes [push.relay]. Scan the Linux QR code in the exact TestFlight app, then approve the matching pairing on Linux. Keep the QR/code/grants private. Keep the phone foreground until registration completes; retain the existing phone node and Mac entry.

```sh
~/.cargo/bin/collied doctor
~/.cargo/bin/collied status
```

Require publisher relay, no APNs key needed, no tags, a running user-owned node and paired phone. Doctor is configuration evidence, not delivery proof. Then background/lock the phone and run one deliberate test:

```sh
~/.cargo/bin/collied apns test
```

Require accepted for the production device and separately confirm notification display on the phone. Use a disposable herdr workspace for approval/control acceptance; no unrelated pane input or global agent policy change.

## Optional matching Omarchy widget after daemon acceptance

```sh
(
set -eu
cd "$collie_download_dir"
curl --fail --location --output CollieTray-0.1.1-omarchy.tar.gz https://github.com/rbstp/collie/releases/download/v0.1.1/CollieTray-0.1.1-omarchy.tar.gz
printf '%s  %s\n' 8325dae685e08d8f4d532267080148c7fd7a2708ab31ec08d72e27f6bb638bcf CollieTray-0.1.1-omarchy.tar.gz | sha256sum --check -
tar -xzf CollieTray-0.1.1-omarchy.tar.gz
cd CollieTray-0.1.1-omarchy
bash install.sh
)
```

Stop on any verification error. The widget installer restarts the Omarchy shell once. Verify status, QR and controls with the matching daemon. No owner Mac installer or service command is part of this Linux handoff.
