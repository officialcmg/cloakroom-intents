import { CHAIN_ICONS } from './chainIconData';

/**
 * The logo of a destination chain on a small round tile. Decorative: the chain's name is
 * always written next to it. A chain without a logo gets its initial on the same tile.
 * The markup is fixed at build time (src/ui/chainIconData.ts); nothing from outside goes in.
 */
export function ChainIcon({ name }: { name: string }) {
  const icon = CHAIN_ICONS[name];
  if (!icon) {
    return (
      <span className="chain-icon chain-icon-initial" aria-hidden="true">
        <span>{name.charAt(0)}</span>
      </span>
    );
  }
  return (
    <span className="chain-icon" aria-hidden="true">
      <svg viewBox={icon.viewBox} focusable="false" dangerouslySetInnerHTML={{ __html: icon.body }} />
    </span>
  );
}
