// What the page says about each stage: the step list, the "now" panel, and whether the
// tab may be closed. Plain words, one idea per sentence.
import type { ErrorCode, Stage, SwapState } from '../engine';
import { formatUnits, formatUsdc, formatZec, USDC_DECIMALS } from '../lib/units';
import { chainName } from '../ui/TokenPicker';
import { formatClock } from '../ui/bits';

export type Safety = 'safe' | 'come_back' | 'keep_open' | 'action';

export interface Group {
  id: string;
  title: (s: SwapState) => string;
  stages: Stage[];
}

export const NORMAL_GROUPS: Group[] = [
  { id: 'pay', title: () => 'Your payment arrives', stages: ['awaiting_deposit', 'deposit_seen', 'deposit_confirmed'] },
  { id: 'price', title: () => 'The maker confirms the price', stages: ['request_sent', 'quote_received', 'accepted'] },
  { id: 'lock', title: () => 'The maker locks the USDC', stages: ['escrow_open'] },
  { id: 'fund', title: () => 'Your ZEC goes to the shared address', stages: ['funding_sent', 'funding_confirming', 'ready'] },
  { id: 'claim', title: () => 'You claim the USDC', stages: ['committing', 'claiming', 'claimed', 'paid_out'] },
  { id: 'deliver', title: (s) => `${s.params.destination.symbol} arrives on ${chainName(s.params.destination.chain)}`, stages: ['exiting', 'exit_retry', 'done'] },
];

export const REFUND_GROUP: Group = { id: 'refund', title: () => 'Your ZEC comes back', stages: ['recovering', 'recovered', 'refunding', 'refunded'] };

export function isRefundPath(s: SwapState): boolean {
  return s.history.some((h) => h.stage === 'refunding' || h.stage === 'recovering');
}

/** The groups to show for this swap: the normal ones, or the ones reached plus the refund group. */
export function groupsFor(s: SwapState): Group[] {
  if (!isRefundPath(s)) return NORMAL_GROUPS;
  const reached = new Set(s.history.map((h) => h.stage));
  const shown = NORMAL_GROUPS.filter((g) => g.stages.some((st) => reached.has(st)));
  return [...shown, REFUND_GROUP];
}

export function groupState(g: Group, s: SwapState): 'done' | 'current' | 'upcoming' | 'stopped' {
  if (g.stages.includes(s.stage)) return s.stage === 'done' || s.stage === 'refunded' ? 'done' : 'current';
  const groups = groupsFor(s);
  const gi = groups.indexOf(g);
  const ci = groups.findIndex((x) => x.stages.includes(s.stage));
  if (gi < ci) {
    // A normal group that the swap left for the refund path without finishing it.
    if (g !== REFUND_GROUP && isRefundPath(s) && gi === ci - 1) {
      const last = g.stages[g.stages.length - 1];
      return s.history.some((h) => h.stage === last) ? 'done' : 'stopped';
    }
    return 'done';
  }
  return 'upcoming';
}

const STAGE_LABEL: Record<Stage, string> = {
  awaiting_deposit: 'Waiting for your payment',
  deposit_seen: 'Payment seen',
  deposit_confirmed: 'Payment confirmed',
  request_sent: 'Request sent to the maker',
  quote_received: 'Firm price received',
  accepted: 'Price accepted, premium paid',
  escrow_open: 'USDC locked, 0.06 NEAR for gas received',
  funding_sent: 'ZEC sent to the shared address',
  funding_confirming: 'Zcash confirmations',
  ready: 'Maker set the escrow to Ready',
  committing: 'Claim step 1: commitment',
  claiming: 'Claim step 2: key half revealed',
  claimed: 'USDC claimed',
  paid_out: 'USDC moved into NEAR Intents',
  exiting: 'Delivery: firm price, signature, submission',
  exit_retry: 'Delivery paused',
  done: 'Delivered',
  refunding: 'Refund being sent',
  refunded: 'Refund sent',
  recovering: 'Taking the ZEC back',
  recovered: 'ZEC back in the swap wallet',
};

export function stageLabel(stage: Stage, s?: SwapState): string {
  if (stage === 'funding_confirming' && s?.funding) return `Zcash confirmations: ${s.funding.confirmations} of ${s.funding.required}`;
  return STAGE_LABEL[stage];
}

export interface NowCopy {
  title: string;
  body: string;
  safety: Safety;
  safetyText: string;
}

// The token the maker pays: USDC, or the stand-in a test network uses (set at boot from
// the network configuration; amounts stay in micro-units either way).
let payToken = 'USDC';
export function setPayToken(symbol: string): void {
  payToken = symbol;
}
export function payTokenLabel(): string {
  return payToken;
}

const usdc = (v?: string) => (v ? `${formatUsdc(BigInt(v))} ${payToken}` : `the ${payToken}`);
const zec = (v?: string) => (v ? `${formatZec(BigInt(v))} ZEC` : 'your ZEC');

export function destAmount(s: SwapState, v: string): string {
  return `${formatUnits(BigInt(v), s.params.destination.decimals)} ${s.params.destination.symbol}`;
}

/** An exit amount: the destination token, or the stand-in token of a direct exit. */
export function exitAmount(s: SwapState, v: string): string {
  const d = s.exit?.direct;
  return d ? `${formatUnits(BigInt(v), d.decimals, { maxFrac: 4 })} ${d.symbol}` : destAmount(s, v);
}

/** What the user receives, in one phrase: "at least 5 USDC on Base", or a direct exit's account. */
export function receiveLine(s: SwapState, standIn?: string): string {
  if (s.params.directExit) return `${s.exit?.direct?.symbol ?? standIn ?? 'tokens'} to the NEAR account ${s.params.directExit.nearAccount}`;
  return `at least ${destAmount(s, s.params.minDestinationAmount)} on ${chainName(s.params.destination.chain)}`;
}

const SAFE_CLOSE = 'You can close this tab. The swap waits and continues when you come back.';

export function nowCopy(s: SwapState): NowCopy {
  const dest = s.params.destination;
  const chain = chainName(dest.chain);
  const comeBack = s.deadlines
    ? `You can close this tab, but come back before ${formatClock(s.deadlines.readyBy)} so the app can claim. If you do not, the maker refunds and your ZEC comes back when you return.`
    : 'You can close this tab, but come back soon so the app can claim.';
  switch (s.stage) {
    case 'awaiting_deposit':
      return { title: 'Waiting for your payment', body: 'Send the amount from your Zcash wallet. It shows here a minute or two after you send.', safety: 'safe', safetyText: SAFE_CLOSE };
    case 'deposit_seen':
      return { title: 'Payment seen', body: 'Your payment is on its way into a Zcash block. That takes about a minute.', safety: 'safe', safetyText: SAFE_CLOSE };
    case 'deposit_confirmed':
      return {
        title: 'Writing to the maker',
        body: 'This browser builds a private Zcash transaction with your request in an encrypted memo. The proof takes about 30 seconds of computing. The page stays usable.',
        safety: 'safe',
        safetyText: SAFE_CLOSE,
      };
    case 'request_sent':
      return { title: 'Waiting for the maker’s firm price', body: 'The maker answers with an encrypted memo of its own. Both transactions need a Zcash block, so this takes two to three minutes.', safety: 'safe', safetyText: SAFE_CLOSE };
    case 'quote_received':
      return { title: 'Accepting the price', body: `The maker offers ${usdc(s.quote?.usdc)}, at or above your minimum. This browser now sends the acceptance with the premium.`, safety: 'safe', safetyText: SAFE_CLOSE };
    case 'accepted':
      return { title: 'Waiting for the maker to lock the USDC', body: `The maker puts ${usdc(s.quote?.usdc)} into the escrow on NEAR. It also sends 0.06 NEAR to a new key made in this browser, to pay for your claim and the payout.`, safety: 'safe', safetyText: SAFE_CLOSE };
    case 'escrow_open':
      return {
        title: 'Sending your ZEC to the shared address',
        body: 'The escrow holds the USDC. This browser now sends your ZEC to an address that needs two key halves to spend: yours and the maker’s.',
        safety: 'safe',
        safetyText: SAFE_CLOSE,
      };
    case 'funding_sent':
    case 'funding_confirming':
      return {
        title: 'Waiting for Zcash confirmations',
        body: `The maker waits for ${s.funding?.required ?? 2} Zcash blocks on your payment to the shared address, then sets the escrow to Ready. Each block takes about 75 seconds.`,
        safety: 'come_back',
        safetyText: comeBack,
      };
    case 'ready':
      return { title: 'Ready to claim', body: 'The maker saw your ZEC and set the escrow to Ready. The claim starts now and takes about a minute.', safety: 'keep_open', safetyText: 'Keep this tab open for about a minute, until the claim is final.' };
    case 'committing':
      return {
        title: 'Claiming your USDC: step 1 of 2',
        body: s.claim?.punishPath
          ? 'The maker did not set the escrow to Ready in time and did not refund. The deadline has passed, so you claim the USDC anyway. First a commitment that reveals nothing.'
          : 'This browser sends a commitment to your key half. It reveals nothing yet. When it is final, the claim follows.',
        safety: 'keep_open',
        safetyText: 'Keep this tab open for about a minute, until the claim is final.',
      };
    case 'claiming':
      return {
        title: 'Claiming your USDC: step 2 of 2',
        body: 'This browser now reveals your key half to the escrow. That gives you the USDC and lets the maker take the ZEC.',
        safety: 'keep_open',
        safetyText: 'Keep this tab open a few more seconds, until the claim is final.',
      };
    case 'claimed':
      return {
        title: 'The USDC is yours',
        body: `The claim is final. This browser now asks the escrow to move ${usdc(s.quote?.usdc)} to the key made in this browser, inside NEAR Intents. The maker’s NEAR pays for this. No other party is needed.`,
        safety: 'safe',
        safetyText: SAFE_CLOSE,
      };
    case 'paid_out':
      return {
        title: 'Your USDC is in NEAR Intents',
        body: `${usdc(s.payout?.usdc)} is in NEAR Intents under the key made in this browser. The delivery to ${chain} starts now.`,
        safety: 'safe',
        safetyText: 'You can close this tab. The USDC stays under your key, and the delivery starts when you come back.',
      };
    case 'exiting': {
      const e = s.exit;
      if (e?.direct) {
        const leave = 'You can close this tab. The tokens stay under your key until the transfer is final.';
        if (e.phase === 'delivering')
          return { title: `Sending ${e.direct.symbol} to ${e.direct.nearAccount}`, body: `The transfer of ${e.amountOut ? exitAmount(s, e.amountOut) : e.direct.symbol} from NEAR Intents executed. The engine checks that it is final.`, safety: 'safe', safetyText: leave };
        return {
          title: `Sending ${e.direct.symbol} to your NEAR account`,
          body: `On this network there is no 1Click, so nothing goes to another chain. This browser signs one transfer of ${e.amountOut ? exitAmount(s, e.amountOut) : e.direct.symbol} from NEAR Intents to ${e.direct.nearAccount}, and the maker’s relay sends it to NEAR. The signature allows exactly this transfer and nothing else.`,
          safety: 'safe',
          safetyText: leave,
        };
      }
      const min = e ? destAmount(s, e.approvedMinimum) : '';
      const leaveOk = 'You can close this tab. The USDC stays under your key until the delivery is final.';
      if (!e || e.phase === 'quoting') {
        const busy = s.lastError?.code === 'EXIT_NO_QUOTE';
        return {
          title: `Getting a firm price for ${dest.symbol}`,
          body: busy
            ? 'NEAR Intents has no price for this pair at this moment. This happens for a few seconds now and then. The engine asks again in 20 seconds.'
            : `This browser asks NEAR Intents for a firm price for your USDC. It goes ahead only at ${min} or more.`,
          safety: 'safe',
          safetyText: leaveOk,
        };
      }
      if (e.phase === 'signing')
        return {
          title: 'Checking and signing the delivery',
          body: `The engine checks the request from NEAR Intents: one transfer of ${usdc(e.amountIn)}, for at least ${destAmount(s, e.minAmountOut ?? e.approvedMinimum)} to your address. Then it signs with your key.`,
          safety: 'safe',
          safetyText: leaveOk,
        };
      return {
        title: `Delivering ${dest.symbol} on ${chain}`,
        body: `NEAR Intents is sending about ${e.amountOut ? destAmount(s, e.amountOut) : dest.symbol} to your address${e.timeEstimateS ? `. It estimates ${e.timeEstimateS} seconds` : ''}. If it cannot deliver at least ${destAmount(s, e.minAmountOut ?? e.approvedMinimum)}, the USDC comes back to your key.`,
        safety: 'safe',
        safetyText: 'You can close this tab. The signed delivery no longer needs it.',
      };
    }
    case 'exit_retry':
      return { title: 'Delivery paused. Your USDC is safe.', body: '', safety: 'action', safetyText: 'Nothing moves until you choose.' };
    case 'done':
      return { title: 'Done', body: '', safety: 'safe', safetyText: '' };
    case 'recovering':
      return {
        title: 'Taking your ZEC back',
        body: 'The maker refunded the escrow. That published the maker’s key half, so this browser can now spend the shared address and take your ZEC back.',
        safety: 'safe',
        safetyText: SAFE_CLOSE,
      };
    case 'recovered':
      return { title: 'Your ZEC is back in the swap’s wallet', body: 'Next it goes to your return address.', safety: 'safe', safetyText: SAFE_CLOSE };
    case 'refunding':
      return s.params.returnAddress
        ? { title: 'Sending your ZEC back', body: `This browser sends the ZEC to your return address ${shortAddr(s.params.returnAddress)}. That takes about two minutes.`, safety: 'safe', safetyText: SAFE_CLOSE }
        : { title: 'Where should your ZEC go?', body: 'Enter a Zcash address below. The refund starts as soon as you do.', safety: 'action', safetyText: 'Your ZEC waits safely in the swap’s wallet until you give an address.' };
    case 'refunded':
      return { title: 'Your ZEC is back', body: '', safety: 'safe', safetyText: '' };
  }
}

export function shortAddr(a: string): string {
  return a.length > 20 ? `${a.slice(0, 10)}…${a.slice(-6)}` : a;
}

/** Why a swap took the refund path, in plain words. */
export function refundReason(code: ErrorCode | undefined, s: SwapState): { title: string; body: string } {
  switch (code) {
    case 'QUOTE_BELOW_MINIMUM':
      return {
        title: 'The maker’s firm price was below your minimum',
        body: `The maker offered ${usdc(s.quote?.usdc)}. Your minimum was ${usdc(s.params.minUsdc)}. So the swap stopped before the premium was paid, and your ZEC goes back to you, less two network fees.`,
      };
    case 'MAKER_TIMEOUT':
      return { title: 'The maker did not answer', body: 'No price came back in time. The swap stopped before the premium was paid, and your ZEC goes back to you, less two network fees.' };
    case 'MAKER_CHECK_FAILED':
      return {
        title: 'The maker’s offer failed a safety check',
        body: `This browser checks every part of the maker’s offer and of the escrow before your ZEC moves. One check failed: ${s.lastError?.message ?? 'see the details'}. Your ZEC did not go to the shared address, and it goes back to you, less network fees${s.history.some((h) => h.stage === 'accepted') ? ' and the premium' : ''}.`,
      };
    case 'MAKER_REFUNDED':
      return {
        title: 'The maker stopped the swap',
        body: 'The maker did not set the escrow to Ready and took its USDC back. That published the maker’s key half, so your ZEC comes back to you. The premium stays with the maker; network fees apply.',
      };
    default:
      return { title: 'The swap stopped', body: 'Your ZEC goes back to you, less network fees.' };
  }
}

export const formatUsdcAmount = (v: string) => `${formatUnits(BigInt(v), USDC_DECIMALS, { maxFrac: 2, minFrac: 2 })} ${payToken}`;
export { zec as formatZecAmount };
