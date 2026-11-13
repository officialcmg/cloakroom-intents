import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { CHAINS } from '../../src/lib/address/chains';
import { selectDestinations } from '../../src/lib/oneclick';
import { DESTINATION_CHAIN_NAMES } from '../../src/lib/chainNames';

describe('destination chain names on the landing page', () => {
  it('are the names of the chain table, in its order', () => {
    expect(DESTINATION_CHAIN_NAMES).toEqual(CHAINS.map((c) => c.name));
  });

  it('match the chains the token list offers (fixture of the real 1Click list, 2026-11-13)', () => {
    const raw = JSON.parse(readFileSync(new URL('../../e2e/fixtures/tokens.json', import.meta.url), 'utf8'));
    const offered = new Set(selectDestinations(raw).destinations.map((t) => t.blockchain));
    expect(offered.size).toBe(DESTINATION_CHAIN_NAMES.length);
    // The landing page's headline says "30+ chains".
    expect(offered.size).toBeGreaterThan(30);
  });
});
