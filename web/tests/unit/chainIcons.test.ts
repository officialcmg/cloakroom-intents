import { describe, expect, it } from 'vitest';
import { DESTINATION_CHAIN_NAMES } from '../../src/lib/chainNames';
import { CHAIN_ICONS } from '../../src/ui/chainIconData';

describe('chain logos on the landing page', () => {
  it('exist for every destination chain, and for no other name', () => {
    expect(Object.keys(CHAIN_ICONS).sort()).toEqual([...DESTINATION_CHAIN_NAMES].sort());
  });

  it('are plain SVG that the CSP allows: no style attributes, no style elements, no outside references', () => {
    for (const [name, icon] of Object.entries(CHAIN_ICONS)) {
      expect(icon.viewBox, name).toMatch(/^-?[\d.]+ -?[\d.]+ [\d.]+ [\d.]+$/);
      expect(icon.body, name).not.toMatch(/style|<script|<image|<foreignObject|\son\w+=|href="(?!#)|url\((?!#)/i);
    }
  });

  it('use ids that no other logo uses', () => {
    const owner = new Map<string, string>();
    for (const [name, icon] of Object.entries(CHAIN_ICONS)) {
      for (const [, id] of icon.body.matchAll(/\sid="([^"]+)"/g)) {
        expect(owner.get(id), `${name} and ${owner.get(id)} both use id ${id}`).toBeUndefined();
        owner.set(id, name);
      }
    }
  });

  it('stay small', () => {
    const bytes = Object.values(CHAIN_ICONS).reduce((n, i) => n + i.body.length, 0);
    expect(bytes).toBeLessThan(40_000);
  });
});
