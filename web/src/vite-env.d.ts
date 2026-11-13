/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** URL of the maker directory JSON. Default: the bundled sample at /maker-directory.json. */
  readonly VITE_DIRECTORY_URL?: string;
  /** Base URL of the NEAR Intents 1Click API. Default: https://1click.chaindefuser.com */
  readonly VITE_ONECLICK_URL?: string;
  /** Network configuration of the real engine: a name (local, testnet) or a URL. Unset: the demo. */
  readonly VITE_ZECRET_CONFIG?: string;
  /** '1': ?config= may name any file or URL, with a warning on the page (local test runs). Off in production builds. */
  readonly VITE_ZECRET_ALLOW_CONFIG_URL?: string;
  /** The mainnet escrow account and its code hash that the page trusts (unset: no mainnet swaps). */
  readonly VITE_MAINNET_ESCROW?: string;
  readonly VITE_MAINNET_ESCROW_CODE_HASH?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
