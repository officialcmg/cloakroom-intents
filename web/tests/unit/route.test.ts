// Landing page at /, the swap app at /swap; old links that name an app setting still open the app.
import { describe, expect, it } from 'vitest';
import { routeFor } from '../../src/app/route';

describe('routeFor', () => {
  it('shows the landing page at / and the app at /swap', () => {
    expect(routeFor('/', '')).toBe('landing');
    expect(routeFor('/swap', '')).toBe('app');
    expect(routeFor('/swap/', '')).toBe('app');
    expect(routeFor('/swap', '?demo=1')).toBe('app');
    expect(routeFor('/anything', '')).toBe('landing');
  });
  it('keeps old links to / with app settings in the app', () => {
    for (const q of ['?demo=1', '?config=testnet', '?config=local', '?speed=20', '?fail=quote-low', '?pause=committing', '?cpu=1', '?directory=/x.json', '?speed=100&fail=exit-fail'])
      expect(routeFor('/', q), q).toBe('app');
  });
  it('ignores parameters that are not app settings', () => {
    expect(routeFor('/', '?utm_source=x')).toBe('landing');
    expect(routeFor('/', '?ref=1')).toBe('landing');
  });
});
