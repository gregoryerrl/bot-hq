import { ToolDefinition } from './types'
import { execSync } from 'child_process'

export const browserTools: ToolDefinition[] = [
  {
    name: 'web_search',
    description: 'Open a Google search in the default browser',
    parameters: {
      type: 'OBJECT',
      properties: {
        query: { type: 'STRING', description: 'The search query to look up on Google' }
      },
      required: ['query']
    },
    destructive: false,
    execute: async (args) => {
      const query = args.query as string
      const encoded = encodeURIComponent(query)
      const url = `https://www.google.com/search?q=${encoded}`

      try {
        execSync(`open "${url}"`, { encoding: 'utf-8', timeout: 10000 })
        return { success: true, query, url }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || 'Failed to open search' }
      }
    }
  },

  {
    name: 'open_url',
    description: 'Open a URL in the default browser',
    parameters: {
      type: 'OBJECT',
      properties: {
        url: { type: 'STRING', description: 'The URL to open in the default browser' }
      },
      required: ['url']
    },
    destructive: false,
    execute: async (args) => {
      const url = args.url as string

      try {
        execSync(`open "${url.replace(/"/g, '\\"')}"`, { encoding: 'utf-8', timeout: 10000 })
        return { success: true, url }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || `Failed to open URL: ${url}` }
      }
    }
  },

  {
    name: 'fetch_page',
    description:
      'Fetch a URL and extract its text content by stripping HTML tags. Returns the text limited to 30000 characters.',
    parameters: {
      type: 'OBJECT',
      properties: {
        url: { type: 'STRING', description: 'The URL to fetch and extract text from' }
      },
      required: ['url']
    },
    destructive: false,
    execute: async (args) => {
      const url = args.url as string

      try {
        const response = await fetch(url, {
          headers: {
            'User-Agent':
              'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36'
          },
          signal: AbortSignal.timeout(15000)
        })

        if (!response.ok) {
          return { error: `HTTP ${response.status}: ${response.statusText}`, url }
        }

        const html = await response.text()

        // Strip HTML tags and decode basic entities
        const text = html
          // Remove script and style blocks entirely
          .replace(/<script[\s\S]*?<\/script>/gi, '')
          .replace(/<style[\s\S]*?<\/style>/gi, '')
          // Remove HTML comments
          .replace(/<!--[\s\S]*?-->/g, '')
          // Remove all HTML tags
          .replace(/<[^>]+>/g, ' ')
          // Decode common HTML entities
          .replace(/&amp;/g, '&')
          .replace(/&lt;/g, '<')
          .replace(/&gt;/g, '>')
          .replace(/&quot;/g, '"')
          .replace(/&#39;/g, "'")
          .replace(/&nbsp;/g, ' ')
          // Collapse whitespace
          .replace(/\s+/g, ' ')
          .trim()

        const truncated = text.length > 30000 ? text.slice(0, 30000) : text

        return { text: truncated, url }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || `Failed to fetch ${url}`, url }
      }
    }
  }
]
