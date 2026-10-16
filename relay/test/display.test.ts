import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { validSend } from '../src/contract.ts';

test('shared Rust and Swift display vectors satisfy the relay allowlist', () => {
  const fixture = JSON.parse(readFileSync(new URL('../../docs/protocol/push-display-vectors.json', import.meta.url), 'utf8'));
  for (const vector of fixture.vectors.filter((value: { payload?: unknown }) => value.payload)) {
    const send = { v: 1, push_type: vector.kind === 1 ? 'alert' : 'liveactivity', priority: 10, expiration: 1791029100, payload: vector.payload };
    assert.equal(validSend(send), true);
    const changed = structuredClone(send);
    if (vector.kind === 1) changed.payload.aps.alert.title = 'Private label';
    else changed.payload.aps['content-state'].title = 'Private label';
    assert.equal(validSend(changed), false);
  }
});
