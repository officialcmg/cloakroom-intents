// The brand in one place, copied from the site's header (web/src/ui/bits.tsx, Logo):
// the open-cape mark of web/public/favicon.svg, and the wordmark "Cloakroom" in the
// grotesk with "Intents" in the serif italic at 1.12 em. To change the mark, replace
// MARK_SVG (any square viewBox works).
window.BRAND = {
  name: 'Cloakroom Intents',
  word1: 'Cloakroom',
  word2: 'Intents',
  url: 'cloakroom.chrismg.dev',
  MARK_SVG: `<svg viewBox="0 0 32 32" xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="bm" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#2a8f72"/><stop offset="1" stop-color="#0d4f3e"/></linearGradient><mask id="bmm"><rect width="32" height="32" fill="#fff"/><circle cx="16" cy="10.6" r="1.7" fill="#000"/><path d="M16 15.2 18.6 26H13.4Z" fill="#000"/></mask></defs><rect width="32" height="32" rx="9.6" fill="url(#bm)"/><rect x="0.5" y="0.5" width="31" height="31" rx="9.1" fill="none" stroke="#fff" stroke-opacity="0.18"/><path mask="url(#bmm)" fill="#fff" stroke="#fff" stroke-width="1.2" stroke-linejoin="round" d="M12.8 7.2H19.2C19.6 13 21.8 19 24.8 24.8 22 25.4 19 25.6 16 25.6S10 25.4 7.2 24.8C10.2 19 12.4 13 12.8 7.2Z"/></svg>`,
};
