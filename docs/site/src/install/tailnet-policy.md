# Tailnet policy

`collied` joins your tailnet as a tagged device, and your phone reaches it on TCP port 8457. The policy file must allow exactly that and nothing more.

## Add the entries

In the Tailscale admin console, open **Access controls** and add:

1. A tag owner for `tag:collie-mac` (and `tag:collie-linux` for a Linux machine), listing your login.
2. A grant from your login to `tag:collie-mac` (and `tag:collie-linux`) on `tcp:8457`.

```hujson
{
  "tagOwners": {
    "tag:collie-mac": ["you@example.com"],
    "tag:collie-linux": ["you@example.com"],
  },
  "grants": [
    { "src": ["you@example.com"], "dst": ["tag:collie-mac"], "ip": ["tcp:8457"] },
    { "src": ["you@example.com"], "dst": ["tag:collie-linux"], "ip": ["tcp:8457"] },
  ],
}
```

Replace `you@example.com` with your login. The [full policy](../reference/tailnet.md#policy-file) also has `tests` that Tailscale runs on every save.

> [!WARNING]
> Grants only add access. Remove the default allow-all grant, and make sure no other rule uses `*`, `autogroup:tagged` or the collie tags. Otherwise other devices can reach the machine, or the machine can reach yours.

## Check it

1. On the **Preview rules** tab, select your user: the only collie entries must be `tag:collie-mac:8457` and `tag:collie-linux:8457` over TCP.
2. Select any other user: neither collie tag may appear.
3. After setup, **Machines** shows the machine with its collie tag and no owner, and the phone under your user with no tag.

`collied setup` prints the entries to add when the tag owner or the grant is missing, and `collied doctor` warns when no device of yours can reach the machine.

Tags, the owner, key expiry and revocation are covered in [Tailnet setup](../reference/tailnet.md).
