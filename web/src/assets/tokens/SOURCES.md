# Token logos: sources and licences

The logos in `src/ui/tokenIconData.ts` are the token issuers' trademarks. The token picker
uses them only to name the tokens that NEAR Intents delivers. Every logo is the token's own
mark, taken from the icon set (except the notes below); none is drawn here. A token that
has no logo in the sets keeps the coloured letter tile.

## Sources

- **web3icons**: npm package `@web3icons/core` 4.0.59, from
  https://github.com/0xa3k5/web3icons. Licence: MIT (the `license` field of the package's
  `package.json`). Paths below are under `dist/svgs/`, `branded` variant. Token metadata
  (coingecko ids, contract addresses) from `@web3icons/common` 0.11.54.
- **cryptocurrency-icons**: npm package `cryptocurrency-icons` 0.18.1. Licence: CC0-1.0
  (`LICENSE.md` in the package).

Neither package is a dependency of the site. The tarballs were fetched from the npm registry
and the logos copied into the data file.

## How a token gets a logo

The lookup key is the token symbol, in upper case. A symbol gets a logo when:

1. web3icons has a token whose coingecko id equals the `coingeckoId` that the 1Click token
   list (`/v0/tokens`, 2026-11-14) gives for every token with that symbol, and whose symbol
   is the same; or
2. the entry was checked by hand (the "Why" column says how).

A wrapped or bridged token gets the logo of the base coin only when the set says it is the
same token. `wBTC` on NEAR gets the WBTC logo, because web3icons lists its NEAR contract
(`2260…c599.factory.bridge.near`) under WBTC. `BTC(OMNI)` is native bitcoin on the Bitcoin
chain, so it uses the BTC logo (alias in the data file). `WETH` and `wNEAR` have no logo:
the sets do not list them as ETH or NEAR.

| Symbol | Source | File | Why |
| --- | --- | --- | --- |
| AAVE | web3icons | tokens/branded/AAVE | coingecko id aave |
| ADA | web3icons | tokens/branded/ADA | coingecko id cardano |
| ADI | web3icons | tokens/branded/ADI | coingecko id adi-token |
| ALEO | web3icons | tokens/branded/ALEO | coingecko id aleo |
| APT | web3icons | tokens/branded/APT | coingecko id aptos |
| ARB | web3icons | tokens/branded/ARB | coingecko id arbitrum |
| AURORA | web3icons | tokens/branded/AURORA | coingecko id aurora-near |
| AVAX | web3icons | tokens/branded/AVAX | coingecko id avalanche-2 |
| BCH | web3icons | tokens/branded/BCH | coingecko id bitcoin-cash |
| BNB | web3icons | tokens/branded/BNB | coingecko id binancecoin |
| BTC | web3icons | tokens/branded/BTC | coingecko id bitcoin |
| CBBTC | web3icons | tokens/branded/CBBTC | coingecko id coinbase-wrapped-btc |
| COW | web3icons | tokens/branded/COW | coingecko id cow-protocol |
| DAI | web3icons | tokens/branded/DAI | coingecko id dai |
| DASH | web3icons | tokens/branded/DASH | coingecko id dash |
| DOGE | cryptocurrency-icons | svg/color/doge.svg | Dogecoin; the web3icons file is over the size limit |
| ETH | web3icons | tokens/branded/ETH | coingecko id ethereum |
| EURE | web3icons | tokens/branded/EURE | Monerium EUR; the set lists the same Gnosis contract 0x420c…3430 |
| FOGO | web3icons | tokens/branded/FOGO | coingecko id fogo |
| FRAX | web3icons | tokens/branded/FRAX | coingecko id frax |
| GMX | web3icons | tokens/branded/GMX | coingecko id gmx |
| GNO | web3icons | tokens/branded/GNO | coingecko id gnosis |
| GRAM | web3icons | tokens/branded/GRAM | the set lists GRAM at the TON native address; the list maps GRAM to the-open-network |
| HAPI | web3icons | tokens/branded/HAPI | coingecko id hapi |
| KNC | web3icons | tokens/branded/KNC | coingecko id kyber-network-crystal |
| LINK | web3icons | tokens/branded/LINK | coingecko id chainlink |
| LTC | web3icons | tokens/branded/LTC | coingecko id litecoin |
| MON | web3icons | networks/branded/monad | the set maps token MON to the Monad network file |
| NEAR | web3icons | tokens/branded/NEAR | coingecko id near |
| NPRO | web3icons | tokens/branded/NPRO | coingecko id npro |
| OP | web3icons | tokens/branded/OP | coingecko id optimism |
| POL | web3icons | tokens/branded/POL | coingecko id polygon-ecosystem-token |
| SAFE | web3icons | tokens/branded/SAFE | coingecko id safe |
| SHIB | web3icons | tokens/branded/SHIB | coingecko id shiba-inu |
| SOL | web3icons | tokens/branded/SOL | coingecko id solana |
| SPX | web3icons | tokens/branded/SPX | coingecko id spx6900 |
| STRK | web3icons | tokens/branded/STRK | coingecko id starknet |
| SUI | web3icons | tokens/branded/SUI | coingecko id sui |
| SWEAT | web3icons | tokens/branded/SWEAT | coingecko id sweatcoin |
| TRX | web3icons | tokens/branded/TRX | coingecko id tron |
| UNI | web3icons | tokens/branded/UNI | coingecko id uniswap |
| USDC | web3icons | tokens/branded/USDC | coingecko id usd-coin |
| USDE | web3icons | tokens/branded/USDE | Ethena USDe; the set lists contract 0x4c9E…68B3 |
| USDT | web3icons | tokens/branded/USDT | coingecko id tether |
| WBTC | web3icons | tokens/branded/WBTC | coingecko id wrapped-bitcoin |
| XAUT | web3icons | tokens/branded/XAUT | coingecko id tether-gold |
| XLM | cryptocurrency-icons | svg/color/xlm.svg | Stellar; web3icons has no branded XLM |
| XPL | web3icons | networks/branded/plasma | the set maps token XPL to the Plasma network file |
| XRP | web3icons | tokens/branded/XRP | coingecko id ripple |
| ZEC | web3icons | tokens/branded/ZEC | coingecko id zcash |

## Changes to the files

- All files went through svgo 3.3.2 (multipass, 2 decimal places). Ids have a prefix per
  logo (`t<symbol>-`), and the page adds a prefix per copy when it draws a logo.
  `class`, `width`, `height` and style attributes are removed (the CSP has `style-src 'self'`).
- The view box is cut to the logo, measured in Chromium. A round coin fills the tile; any
  other mark fills 72 % of it, centred on the light disc.
- XRP: the branded mark is white. Its fill is changed to the dark brand colour `#23292F` so
  that it shows on the light tile (as for the XRP Ledger chain logo).
- Logos over 6 KB after svgo are left out and keep the letter tile: MOG (12.3 KB), PEPE
  (7.2 KB). The web3icons DOGE file is 16.7 KB; DOGE uses the cryptocurrency-icons file.
