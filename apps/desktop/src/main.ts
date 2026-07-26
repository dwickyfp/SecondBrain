import { mount } from 'svelte';
import App from './App.svelte';
import './styles.css';
import { reportReleaseReadiness } from './lib/release-readiness';

mount(App, { target: document.getElementById('app')! });

const pageLoaded = document.readyState === 'complete'
  ? Promise.resolve()
  : new Promise<void>((resolve) => window.addEventListener('load', () => resolve(), { once: true }));
void pageLoaded
  .then(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))))
  .then(reportReleaseReadiness)
  .catch((error) => console.error('Release readiness handshake failed', error));
