import { lazy, StrictMode, Suspense, useEffect, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import './styles/tokens.css';
import './styles/app.css';
import { useRoute } from './app/route';
import { Landing } from './screens/Landing';
import { Header } from './screens/Chrome';
import { Spinner } from './ui/bits';

// The swap app is a separate chunk. Its code, the engine worker and the WebAssembly load
// only when the user opens /swap (see src/app/route.ts).
const App = lazy(() => import('./App'));

function AppLoading() {
  return (
    <div className="app">
      <Header />
      <main id="main" className="main">
        <div className="loading" role="status">
          <Spinner />
          <p>Opening the swap app…</p>
        </div>
      </main>
    </div>
  );
}

function Root() {
  const route = useRoute();
  // After a move between the landing page and the app, focus the new page's heading.
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    const h = document.querySelector<HTMLElement>('main h1');
    if (h) {
      h.setAttribute('tabindex', '-1');
      h.focus({ preventScroll: true });
    }
  }, [route]);
  return route === 'app' ? (
    <Suspense fallback={<AppLoading />}>
      <App />
    </Suspense>
  ) : (
    <Landing />
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
