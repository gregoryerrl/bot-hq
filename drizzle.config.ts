import { defineConfig } from 'drizzle-kit'

export default defineConfig({
  schema: './electron/memory/schema.ts',
  out: './drizzle',
  dialect: 'sqlite'
})
