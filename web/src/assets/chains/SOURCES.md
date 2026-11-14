# Chain logos: sources and licences

The logos in `src/ui/chainIconData.ts` are the chains' trademarks. The landing page uses
them only to name the chains that the swap form supports. Every logo is the chain's own
mark, taken unchanged from the icon set (except the two notes below); none is drawn here.

## Sources

- **web3icons**: npm package `@web3icons/core` 4.0.59, from
  https://github.com/0xa3k5/web3icons. Licence: MIT (the `license` field of the package's
  `package.json`; the package and the repository have no separate licence file).
  Path below is under `dist/svgs/`, `branded` variant.
- **cryptocurrency-icons**: npm package `cryptocurrency-icons` 0.18.1. Licence: CC0-1.0
  (`LICENSE.md` in the package).

| Chain | Source | File |
| --- | --- | --- |
| Ethereum | web3icons | networks/branded/ethereum |
| Base | web3icons | networks/branded/base |
| Arbitrum | web3icons | networks/branded/arbitrum-one |
| Optimism | web3icons | networks/branded/optimism |
| Polygon | web3icons | networks/branded/polygon |
| BNB Chain | web3icons | networks/branded/binance-smart-chain |
| Avalanche | web3icons | networks/branded/avalanche |
| Gnosis | web3icons | networks/branded/gnosis |
| Scroll | web3icons | networks/branded/scroll |
| Abstract | web3icons | networks/branded/abstract |
| Berachain | web3icons | networks/branded/berachain |
| Monad | web3icons | networks/branded/monad |
| X Layer | web3icons | networks/branded/x-layer |
| Plasma | web3icons | networks/branded/plasma |
| Robinhood Chain | web3icons | networks/branded/robinhood (network id `robinhood`, chain id 4663) |
| ADI | web3icons | tokens/branded/ADI (web3icons maps network `adi-chain` to this file) |
| Hyperliquid | web3icons | networks/branded/hyper-evm |
| Bitcoin | web3icons | networks/branded/bitcoin |
| Litecoin | web3icons | networks/branded/litecoin |
| Dogecoin | cryptocurrency-icons | svg/color/doge.svg (the web3icons file is 25 KB) |
| Bitcoin Cash | web3icons | tokens/branded/BCH |
| Dash | web3icons | tokens/branded/DASH |
| Solana | web3icons | networks/branded/solana |
| Fogo | web3icons | tokens/branded/FOGO (web3icons maps network `fogo` to this file) |
| NEAR | web3icons | networks/branded/near-protocol |
| Aptos | web3icons | networks/branded/aptos |
| Movement | web3icons | networks/branded/movement |
| Sui | web3icons | networks/branded/sui |
| Starknet | web3icons | networks/branded/starknet |
| TON | web3icons | networks/branded/ton |
| Tron | web3icons | networks/branded/tron |
| Stellar | web3icons | networks/branded/stellar |
| XRP Ledger | web3icons | networks/branded/xrp |
| Cardano | web3icons | networks/branded/cardano |
| Aleo | web3icons | tokens/branded/ALEO (web3icons maps network `aleo` to this file) |

## Changes to the files

- All files went through svgo 3.3.2 (multipass, 2 decimal places). Ids have a prefix per
  logo. `class`, `width`, `height` and style attributes are removed.
- XRP Ledger: the branded mark is white. Its fill is changed to the dark brand colour
  `#23292F` so that it shows on the light tile.
- Dogecoin: the view box is widened so that the coin has the same size as the other logos.
