// The interface between the page and the swap engine.
//
// The engine runs in a Web Worker. The page talks to it only through `Engine`, which is
// carried over postMessage by `rpc.ts`. Two implementations exist or will exist:
//   - the simulated engine (`sim/`), used until the real one lands and for tests;
//   - the real engine (Rust compiled to WebAssembly), which implements `EngineBackend`
//     in its own worker. See ENGINE_API.md.
//
// All amounts are strings of integer base units (zats for ZEC, micro-USDC for USDC, the
// token's own base unit for the destination). All times are Unix milliseconds.

export type Network = 'mainnet' | 'testnet' | 'regtest';
export type EngineKind = 'simulated' | 'wasm';

// ---------------------------------------------------------------- stages

/**
 * Where a swap is. The normal path runs top to bottom (docs/DESIGN.md §3). The last
 * six are the failure paths.
 */
export const STAGES = [
  'awaiting_deposit', // the temporary wallet waits for the user's payment
  'deposit_seen', // the payment is in the mempool or a block, not yet spendable
  'deposit_confirmed', // the payment can be spent
  'request_sent', // request memo sent to the maker (first shielded send)
  'quote_received', // the maker's firm quote arrived and is at or above the minimum
  'accepted', // acceptance memo and premium sent (second shielded send)
  'escrow_open', // the maker locked USDC in the escrow and sent the NEAR stipend
  'funding_sent', // ZEC sent to the shared address (third shielded send)
  'funding_confirming', // the funding gains confirmations: funding.confirmations of funding.required
  'ready', // the maker saw the ZEC and set the escrow to Ready
  'committing', // commit_claim sent; the tab must stay open from here until `claimed`
  'claiming', // claim(a) sent after the commit is final
  'claimed', // the claim is final; the USDC is the user's
  'paid_out', // the escrow paid the USDC into NEAR Intents under the user's NEAR key
  'exiting', // the signed exit intent was submitted
  'done', // the destination token arrived
  'refunding', // ZEC is being sent back to the return address
  'refunded', // ZEC arrived back at the return address
  'recovering', // the maker refunded the escrow; the engine takes the ZEC back from the shared address
  'recovered', // the ZEC is back in the temporary wallet
  'exit_retry', // the exit failed or its price fell below the minimum; the USDC is safe; waiting for a decision
] as const;

export type Stage = (typeof STAGES)[number];

/** Stages after which nothing more happens unless the user starts something new. */
export const TERMINAL_STAGES: readonly Stage[] = ['done', 'refunded'];

// ---------------------------------------------------------------- errors

export const ERROR_CODES = [
  'INIT_FAILED', // the engine could not start (no WebAssembly, no storage, key build failed)
  'NOT_READY', // a call came before init
  'INVALID_PARAMS', // createSwap or exit arguments are wrong
  'INVALID_ADDRESS', // an address failed the engine's own check
  'SWAP_EXISTS', // this browser already holds an unfinished swap
  'NO_SWAP', // the call needs a swap and there is none
  'WRONG_STAGE', // the call is not allowed in the current stage
  'RECOVERY_INVALID', // the recovery file cannot be read
  'RECOVERY_MISMATCH', // the recovery file is for another network or conflicts with the stored swap
  'QUOTE_BELOW_MINIMUM', // the maker's firm quote was below params.minUsdc
  'MAKER_TIMEOUT', // the maker did not answer the request in time
  'MAKER_REFUNDED', // the maker refunded the escrow instead of setting it ready
  'EXIT_FAILED', // the exit through NEAR Intents did not complete
  'EXIT_BELOW_MINIMUM', // the exit quote is below params.minDestinationAmount
  'EXIT_QUOTE_EXPIRED', // the exit quote's deadline passed
  'NETWORK', // a server could not be reached; the engine retries by itself
  'STORAGE', // the browser refused to store the swap
  'INTERNAL', // a bug; the message has details
] as const;

export type ErrorCode = (typeof ERROR_CODES)[number];

export interface EngineErrorData {
  code: ErrorCode;
  message: string;
  /** True if the engine retries by itself and the user need not act. */
  transient?: boolean;
}

export class EngineError extends Error {
  readonly code: ErrorCode;
  readonly transient?: boolean;
  constructor(code: ErrorCode, message: string, transient?: boolean) {
    super(message);
    this.name = 'EngineError';
    this.code = code;
    this.transient = transient;
  }
  toData(): EngineErrorData {
    return { code: this.code, message: this.message, transient: this.transient };
  }
  static from(data: EngineErrorData): EngineError {
    return new EngineError(data.code, data.message, data.transient);
  }
}

// ---------------------------------------------------------------- configuration and init

export interface SimOptions {
  /** Time multiplier: 1 is real time, 20 runs a 10-minute swap in 30 s. */
  speed: number;
  /** Failure to inject. */
  fail?: SimFailure;
  /** Busy-loop the worker during simulated proofs, as the real prover does. */
  burnCpu?: boolean;
}

export const SIM_FAILURES = ['quote-low', 'no-quote', 'maker-quiet', 'maker-gone', 'exit-fail'] as const;
export type SimFailure = (typeof SIM_FAILURES)[number];

export interface EngineConfig {
  network: Network;
  /** The maker directory as fetched by the page. The engine checks its signature. */
  directory: unknown;
  lightwalletdUrl?: string;
  nearRpcUrl?: string;
  /** Only read by the simulated engine. */
  sim?: SimOptions;
}

export type InitPhase = 'starting' | 'building_keys' | 'ready' | 'failed';

export interface InitStatus {
  /** Which engine answered. The page shows the demo notice and controls only for 'simulated'. */
  engine: EngineKind;
  phase: InitPhase;
  /** When the current phase started. */
  startedAt: number;
  /** Expected length of the current phase, for a progress bar the page draws by itself. */
  estimatedMs?: number;
  /** Real time divided by this gives the time the page should show (1 for the real engine). */
  timeScale: number;
  error?: EngineErrorData;
}

// ---------------------------------------------------------------- the swap

export interface Destination {
  /** 1Click asset id, for example nep141:sol.omft.near */
  assetId: string;
  symbol: string;
  /** 1Click blockchain id, for example sol */
  chain: string;
  decimals: number;
  address: string;
}

export interface CreateSwapParams {
  /** What the user pays into the temporary wallet. */
  amountZats: string;
  destination: Destination;
  /** Unified address with an Orchard receiver, for refunds and leftovers. */
  returnAddress?: string;
  /** The engine accepts the maker's firm quote only at or above this (micro-USDC). */
  minUsdc: string;
  /** The exit does not run below this without the user's consent (destination base units). */
  minDestinationAmount: string;
  /** For display: what the page showed when the user agreed. */
  shown: { usdcExpected: string; destinationExpected: string; premiumZats: string; networkFeeZats: string };
}

export type Chain = 'zcash' | 'near' | 'destination';

export interface TxRef {
  /** Stable key, for example "funding". */
  id: string;
  chain: Chain;
  label: string;
  hash: string;
  url?: string;
  at: number;
}

export interface Activity {
  kind: 'proving' | 'syncing' | 'broadcasting' | 'signing' | 'waiting';
  /** Plain words, for example "Building the proof for your payment to the shared address". */
  label: string;
  startedAt: number;
  estimatedMs?: number;
}

export interface SwapState {
  version: 1;
  id: string;
  engine: EngineKind;
  network: Network;
  createdAt: number;
  stage: Stage;
  stageAt: number;
  history: { stage: Stage; at: number }[];
  params: CreateSwapParams;
  deposit: {
    address: string;
    /** ZIP 321 URI for the amount still owed. */
    uri: string;
    expectedZats: string;
    receivedZats: string;
    confirmations: number;
    requiredConfirmations: number;
  };
  quote?: { usdc: string; premiumZats: string; expiresAt: number };
  escrow?: { contract: string; swapId: string; takerAccount: string; stipendYocto: string };
  funding?: { amountZats: string; confirmations: number; required: number };
  claim?: { punishPath: boolean };
  payout?: { usdc: string; intentsAccount: string };
  exit?: {
    attempts: number;
    amountIn?: string;
    amountOut?: string;
    minAmountOut?: string;
    depositAddress?: string;
    /** A quote the engine refused (below minimum or expired), offered to the user. */
    offered?: { amountOut: string; minAmountOut: string };
    deliveredAmount?: string;
  };
  refund?: { reason: ErrorCode; amountZats?: string; toAddress?: string };
  leftover?: { amountZats: string; toAddress?: string };
  /** What the engine needs from the user, if anything. */
  needs?: 'return_address' | 'exit_decision' | null;
  activity?: Activity | null;
  txs: TxRef[];
  /** Deadlines from the quote (Unix ms): until when the maker may set ready, and when the punish path opens. */
  deadlines?: { readyBy: number; punishFrom: number };
  recoveryExportedAt?: number;
  lastError?: EngineErrorData | null;
}

type Patchable = Omit<SwapState, 'version' | 'id' | 'engine' | 'network' | 'createdAt' | 'stage' | 'stageAt' | 'history' | 'txs'>;

/** Facts that changed with a stage event. Object fields merge one level deep. */
export type SwapPatch = { [K in keyof Patchable]?: Patchable[K] extends object | undefined | null ? Partial<NonNullable<Patchable[K]>> | null : Patchable[K] } & {
  addTxs?: TxRef[];
};

// ---------------------------------------------------------------- events

export type EngineEvent =
  | { type: 'init'; init: InitStatus }
  | { type: 'swap'; swap: SwapState | null }
  | { type: 'stage'; swapId: string; stage: Stage; at: number; patch?: SwapPatch }
  | { type: 'activity'; swapId: string; activity: Activity | null }
  | { type: 'error'; swapId?: string; error: EngineErrorData };

// ---------------------------------------------------------------- recovery and exit

export interface RecoveryFile {
  fileName: string;
  mimeType: string;
  /** The file's text. The page saves it as is and never reads the secrets inside. */
  text: string;
}

export interface ExitQuoteInput {
  amountIn: string;
  amountOut: string;
  minAmountOut: string;
  /** Required by the real engine: the 1Click deposit address that the signed transfer pays. */
  depositAddress?: string;
  /** ISO time after which 1Click no longer honours the quote. */
  deadline: string;
}

export interface ExitRequest {
  quote: ExitQuoteInput;
  /** The user saw the lower amount and agreed to it. */
  acceptBelowMinimum?: boolean;
}

// ---------------------------------------------------------------- the interface

export interface Engine {
  /** Starts the engine and the proving-key build. Resolves when the build has started, not finished. */
  init(config: EngineConfig): Promise<InitStatus>;
  /** All events. Returns a function that unsubscribes. */
  subscribe(listener: (event: EngineEvent) => void): () => void;
  /** Loads the stored swap, if any, and continues it. Call once after init. */
  resume(): Promise<SwapState | null>;
  getState(): Promise<SwapState | null>;
  /** Makes the temporary wallet and the swap's keys, stores them, and returns the deposit address. */
  createSwap(params: CreateSwapParams): Promise<SwapState>;
  /** Sets or changes the address for refunds and leftovers. */
  setReturnAddress(address: string): Promise<SwapState>;
  exportRecovery(): Promise<RecoveryFile>;
  /** Restores a swap from a recovery file and continues it. */
  importRecovery(text: string): Promise<SwapState>;
  /** Signs and submits the exit for the given 1Click quote. Allowed in paid_out and exit_retry. */
  exit(request: ExitRequest): Promise<SwapState>;
  /** Deletes the stored swap. Allowed before any payment arrives, and after done or refunded. */
  forget(): Promise<void>;
  dispose(): void;
}

/** Controls that only the simulated engine has. */
export interface SimControls {
  /** Pretends the user's wallet paid. Without an amount it pays what is owed. */
  pay(amountZats?: string): Promise<void>;
  /** Moves the current wait forward to its end. */
  skip(): Promise<void>;
  /** Deletes the swap's local keys but keeps the simulated chain, as a cleared browser would. */
  clearLocal(): Promise<void>;
}

/** The method names that cross the transport. */
export type EngineMethod = Exclude<keyof Engine, 'subscribe' | 'dispose'>;
export type SimMethod = `sim.${keyof SimControls}`;
