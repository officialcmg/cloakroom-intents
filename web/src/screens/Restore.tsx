import { useRef, useState } from 'react';
import type { EngineError } from '../engine';
import { appStore } from '../app/useZecret';
import { Icon } from '../ui/Icon';
import { Notice, Spinner } from '../ui/bits';

const MAX_BYTES = 1_000_000;

export function RestoreScreen({ onCancel, onRestored }: { onCancel: () => void; onRestored: () => void }) {
  const input = useRef<HTMLInputElement>(null);
  const [drag, setDrag] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ title: string; text: string }>();

  const load = async (file: File | undefined) => {
    if (!file) return;
    setError(undefined);
    if (file.size > MAX_BYTES) {
      setError({ title: 'This file is too large', text: 'A recovery file is a few kilobytes. Choose the file named cloakroom-intents-recovery-….json.' });
      return;
    }
    setBusy(true);
    try {
      const text = await file.text();
      await appStore.engine!.importRecovery(text);
      onRestored();
    } catch (e) {
      const err = e as EngineError;
      const title =
        err.code === 'SWAP_EXISTS'
          ? 'Another swap is in progress here'
          : err.code === 'RECOVERY_MISMATCH'
            ? 'This file is for another network'
            : 'This file cannot be used';
      setError({ title, text: err.message });
    } finally {
      setBusy(false);
      if (input.current) input.current.value = '';
    }
  };

  return (
    <div className="stack-lg narrow">
      <section className="card card-main stack" aria-labelledby="restore-title">
        <span className="seal-icon" aria-hidden="true">
          <Icon name="file" size={22} />
        </span>
        <h1 id="restore-title" className="h1">
          Restore a swap
        </h1>
        <p className="lede">Choose the recovery file you saved when the swap started. The swap continues from where it is now, or your ZEC comes back if it stopped.</p>
        <label
          className={`dropzone${drag ? ' is-drag' : ''}`}
          onDragOver={(e) => {
            e.preventDefault();
            setDrag(true);
          }}
          onDragLeave={() => setDrag(false)}
          onDrop={(e) => {
            e.preventDefault();
            setDrag(false);
            void load(e.dataTransfer.files[0]);
          }}
        >
          <input ref={input} type="file" accept="application/json,.json" className="sr-only" onChange={(e) => void load(e.target.files?.[0])} data-testid="recovery-input" />
          {busy ? <Spinner label="Reading the file" /> : <Icon name="upload" size={26} />}
          <span className="dropzone-title">{busy ? 'Reading the file…' : 'Choose the recovery file'}</span>
          <span className="hint">or drop it here. It is named cloakroom-intents-recovery-….json</span>
        </label>
        {error && (
          <Notice tone="danger" title={error.title} role="alert">
            {error.text}
          </Notice>
        )}
        <p className="fineprint">The file stays in this browser. Nothing is uploaded.</p>
        <button type="button" className="btn btn-link" onClick={onCancel}>
          Back
        </button>
      </section>
    </div>
  );
}
