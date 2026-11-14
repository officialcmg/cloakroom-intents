import { CHAIN_ICONS } from './chainIconData';

/**
 * The logo of a destination chain on a small round tile. Decorative: the chain's name is
 * always written next to it. Every destination chain has a logo (tests/unit/chainIcons.test.ts).
 * The markup is fixed at build time (src/ui/chainIconData.ts); nothing from outside goes in.
 */
export function ChainIcon({ name }: { name: string }) {
  const icon = CHAIN_ICONS[name];
  if (!icon) return null;
  return (
    <span className="chain-icon" aria-hidden="true">
      <svg viewBox={icon.viewBox} focusable="false" dangerouslySetInnerHTML={{ __html: icon.body }} />
    </span>
  );
}
