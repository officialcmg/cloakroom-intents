// The start-failure screen says the real cause (App.tsx, StartFailure).
import { describe, expect, it } from 'vitest';
import { failureCopy } from '../../src/app/failureCopy';

const KEY = 'ed25519:4aQfVk7LxZzASG5mLRsjDn1AxkzsCDjjmtf5q6ApZyHH';
const BROWSER_HINT = /may block Web Workers/;

describe('failureCopy', () => {
  it('a failed maker check says the maker could not be verified, not that the browser blocks something', () => {
    // The message the hosted site showed (src/engine/real/realEngine.ts, init; crates/zecret-core/src/taker.rs, verify_maker).
    const message = `The maker's directory entry did not pass the check: maker key ${KEY} is not a key of maker.zecret-3f109eea1e.testnet`;
    const c = failureCopy({ kind: 'engine', message });
    expect(c.cause).toBe('maker_unverified');
    expect(c.title).toBe('The maker could not be verified');
    expect(c.body).toMatch(/does not swap with a maker it cannot verify/);
    expect(c.body).toMatch(/Nothing was sent/);
    expect(c.next).toMatch(/Try again later/);
    expect(c.next).not.toMatch(BROWSER_HINT);
    expect(c.detail).toBe(message);
  });

  it('a maker check that could not reach NEAR asks to check the connection', () => {
    const c = failureCopy({
      kind: 'engine',
      message: "The maker's directory entry did not pass the check: NEAR RPC query: POST https://rpc.testnet.fastnear.com: no answer (network error or CORS)",
    });
    expect(c.cause).toBe('maker_unreachable');
    expect(c.title).toBe('The maker could not be checked');
    expect(c.next).toMatch(/connection/);
    const c502 = failureCopy({ kind: 'engine', message: "The maker's directory entry did not pass the check: NEAR RPC query: HTTP 502 Bad Gateway" });
    expect(c502.cause).toBe('maker_unreachable');
  });

  it('other problems with the maker entry are maker problems too', () => {
    for (const message of [
      'The maker directory is not valid: maker.requestKey must be 32 bytes of lowercase hex',
      'The maker directory is for mainnet; this page runs on testnet.',
      'The maker names the escrow evil.testnet; this page trusts escrow2.zecret-3f109eea1e.testnet.',
      'The maker directory has no signed entry (maker.nearPublicKey, maker.epoch, signature).',
      `The maker's directory entry did not pass the check: maker directory entry is not signed by ${KEY}`,
    ]) {
      const c = failureCopy({ kind: 'engine', message });
      expect(c.cause, message).toBe('maker_unverified');
      expect(c.next).not.toMatch(BROWSER_HINT);
    }
  });

  it('shows the browser hint only when the browser is a plausible cause', () => {
    expect(failureCopy({ kind: 'engine', message: 'no IndexedDB' }).next).toMatch(BROWSER_HINT);
    expect(failureCopy({ kind: 'engine', message: '' }).title).toBe('The swap engine could not start');
    const wasm = failureCopy({ kind: 'engine', message: 'The WebAssembly engine did not load: Failed to fetch' });
    expect(wasm.cause).toBe('engine_download');
    expect(wasm.next).toMatch(/connection/);
    for (const kind of ['directory', 'config'] as const) expect(failureCopy({ kind, message: 'x answered 503' }).next).not.toMatch(BROWSER_HINT);
  });

  it('names the other causes', () => {
    expect(failureCopy({ kind: 'directory', message: 'The maker directory answered 503' }).title).toBe('The maker’s details did not load');
    expect(failureCopy({ kind: 'config', message: 'The network configuration /config/testnet.json answered 404' }).cause).toBe('config');
    const refused = failureCopy({ kind: 'config', message: 'This page loads only its own network settings (local, testnet); it does not load "https://a.example/c.json" from a link.' });
    expect(refused.cause).toBe('config_refused');
    expect(refused.retry).toBe(false);
    expect(failureCopy({ kind: 'engine', message: 'The swap engine stopped again: unreachable' }).cause).toBe('engine_crashed');
  });

  it('does not say "nothing was sent" when this browser holds a swap', () => {
    const c = failureCopy({ kind: 'directory', message: 'The maker directory answered 503' }, { hasSwap: true });
    expect(c.body).not.toMatch(/Nothing was sent/);
    expect(c.body).toMatch(/stored in this browser/);
  });
});
