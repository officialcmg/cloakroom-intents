import { useMemo } from 'react';
import QRCode from 'qrcode';

/** A QR code as one SVG path. Always dark on light, in both themes, so every wallet camera reads it. */
export function QrCode({ text, label, size = 232 }: { text: string; label: string; size?: number }) {
  const { path, count } = useMemo(() => {
    const qr = QRCode.create(text, { errorCorrectionLevel: 'M' });
    const n = qr.modules.size;
    const data = qr.modules.data;
    let d = '';
    for (let y = 0; y < n; y++) {
      let x = 0;
      while (x < n) {
        if (data[y * n + x]) {
          const start = x;
          while (x < n && data[y * n + x]) x++;
          d += `M${start} ${y}h${x - start}v1h-${x - start}z`;
        } else {
          x++;
        }
      }
    }
    return { path: d, count: n };
  }, [text]);
  const quiet = 3;
  const box = count + quiet * 2;
  return (
    <svg className="qr" width={size} height={size} viewBox={`${-quiet} ${-quiet} ${box} ${box}`} role="img" aria-label={label} shapeRendering="crispEdges">
      <rect x={-quiet} y={-quiet} width={box} height={box} fill="var(--qr-paper)" />
      <path d={path} fill="var(--qr-ink)" />
    </svg>
  );
}
