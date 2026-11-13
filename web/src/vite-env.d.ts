/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** URL of the maker directory JSON. Default: the bundled sample at /maker-directory.json. */
  readonly VITE_DIRECTORY_URL?: string;
  /** Base URL of the NEAR Intents 1Click API. Default: https://1click.chaindefuser.com */
  readonly VITE_ONECLICK_URL?: string;
  /** Network configuration of the real engine: a name (local, testnet) or a URL. Unset: the demo. */
  readonly VITE_ZECRET_CONFIG?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
