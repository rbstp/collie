# Architecture decision records

Decisions about the code, recorded where every place they shape can find them. A comment in the code may point here with a bare `ADR-NNNN` where an invariant is security relevant or the code looks wrong without it ([ADR-0001](0001-comments-and-adrs.md)).

A record has a status, a date, Context, Decision, Consequences and the code it governs. A new one takes the next number and a line below; a decision that changes is edited in place, or superseded by a new record that says which one it replaces.

| ADR | Decision |
|---|---|
| [0001](0001-comments-and-adrs.md) | Decisions live in ADRs; comments are data, directives or pointers, held down by a ratchet |
| [0002](0002-ci-jobs-and-caches.md) | CI runs jobs by changed paths, caches native builds and boots the simulator early |
| [0003](0003-write-tokens-never-run-pull-request-code.md) | Workflows that hold write tokens never run pull request code |
| [0004](0004-release-signing-and-notices.md) | Release signing, versioning and third-party notices |
| [0005](0005-libghostty-vt-builds.md) | libghostty-vt is built from pinned tarballs with a checked Zig, a private cache and ReleaseSafe in collied |
| [0006](0006-desktop-bars-and-the-control-socket.md) | The desktop bars speak collied's control socket and confirm pairings on the machine |
| [0007](0007-documentation-screenshots-and-site.md) | Documentation screenshots come from the demo on a fixed simulator; the site is staged and checked |
| [0008](0008-relay-storage-bounds-and-attestation.md) | The relay checks App Attest flags strictly and bounds its storage in guarded batches |
| [0009](0009-antigravity-cli.md) | Antigravity CLI is read like the other agents, through its own hook, files and layout |
| [0010](0010-startup-dialog-keys.md) | Keys reach a silent agent's startup dialog only while it shows |
| [0011](0011-code-mode.md) | Code Mode is set on Claude Code's own command, or as a setting through herdr |
| [0012](0012-question-forms.md) | A question form is answered only when its call or the screen proves it whole |
| [0013](0013-rex-resume.md) | A Claude Code in Rex resumes once, beside its restored shell, which closes only once it runs |
