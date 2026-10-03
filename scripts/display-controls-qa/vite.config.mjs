import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';
const stub = path.resolve('scripts/display-controls-qa/tauri-stub.js');
export default defineConfig({ plugins: [react()], resolve: { alias: { '@tauri-apps/api/window': stub, '@tauri-apps/api/core': stub, '@tauri-apps/api/event': stub } }, optimizeDeps: { entries: ['scripts/display-controls-qa/main.tsx'] }, server: { watch: { ignored: ['**/src-tauri/**'] }, host: '127.0.0.1', port: 1427, strictPort: true } });
