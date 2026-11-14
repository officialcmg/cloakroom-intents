import { describe, expect, it } from 'vitest';
import { CHAIN_ICONS } from '../../src/ui/chainIconData';
import { scopeIds } from '../../src/ui/TokenAvatar';
import { TOKEN_ALIASES, TOKEN_ICONS, tokenIcon } from '../../src/ui/tokenIconData';

describe('token logos in the token picker', () => {
  it('find the logo of the major tokens by symbol, in any case', () => {
    for (const [symbol, key] of [
      ['USDC', 'USDC'],
      ['usdc', 'USDC'],
      ['ETH', 'ETH'],
      ['BTC', 'BTC'],
      ['BTC(OMNI)', 'BTC'],
      ['SOL', 'SOL'],
      ['NEAR', 'NEAR'],
      ['ZEC', 'ZEC'],
      ['USDT', 'USDT'],
      ['wBTC', 'WBTC'],
    ]) {
      expect(tokenIcon(symbol), symbol).toBe(TOKEN_ICONS[key]);
      expect(tokenIcon(symbol), symbol).toBeDefined();
    }
    expect(tokenIcon('USDC')).not.toBe(tokenIcon('USDT'));
  });

  it('give no logo when the sets have none for that token, so the letter tile shows', () => {
    // WETH and wNEAR: the sets do not treat them as ETH or NEAR. USDT0: not in the sets.
    for (const symbol of ['WETH', 'wNEAR', 'USDT0', 'sUSDC', 'xBTC', 'NOT-A-TOKEN', '', 'constructor', '__proto__', 'toString']) {
      expect(tokenIcon(symbol), symbol).toBeUndefined();
    }
  });

  it('aliases point at logos that exist', () => {
    for (const key of Object.values(TOKEN_ALIASES)) expect(TOKEN_ICONS[key], key).toBeDefined();
  });

  it('are plain SVG that the CSP allows: no style attributes, no style elements, no outside references', () => {
    for (const [name, icon] of Object.entries(TOKEN_ICONS)) {
      expect(name).toBe(name.toUpperCase());
      expect(icon.viewBox, name).toMatch(/^-?[\d.]+ -?[\d.]+ [\d.]+ [\d.]+$/);
      expect(icon.body, name).not.toMatch(/style|<script|<image|<foreignObject|\son\w+=|href="(?!#)|url\((?!#)/i);
    }
  });

  it('use ids that no other token or chain logo uses', () => {
    const owner = new Map<string, string>();
    for (const [name, icon] of [...Object.entries(TOKEN_ICONS), ...Object.entries(CHAIN_ICONS)]) {
      for (const [, id] of icon.body.matchAll(/\sid="([^"]+)"/g)) {
        expect(owner.get(id), `${name} and ${owner.get(id)} both use id ${id}`).toBeUndefined();
        owner.set(id, name);
      }
    }
  });

  it('get a prefix per copy, for ids and the references to them', () => {
    const body = '<linearGradient id="tsol-a"/><path fill="url(#tsol-a)"/><use href="#tsol-a"/>';
    expect(scopeIds(body, 'x1-')).toBe('<linearGradient id="x1-tsol-a"/><path fill="url(#x1-tsol-a)"/><use href="#x1-tsol-a"/>');
  });

  it('stay small: each logo under 6 KB', () => {
    for (const [name, icon] of Object.entries(TOKEN_ICONS)) expect(icon.body.length, name).toBeLessThanOrEqual(6144);
  });
});
