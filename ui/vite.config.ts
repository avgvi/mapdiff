import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    // While developing the page, the API still comes from `mapdiff serve`.
    proxy: {
      '/api': 'http://127.0.0.1:7878',
    },
  },
})
