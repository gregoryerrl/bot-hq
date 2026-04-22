export {}

declare global {
  interface Window {
    api: {
      send: (channel: string, data: unknown) => void
      on: (channel: string, callback: (...args: any[]) => void) => () => void
      invoke: (channel: string, ...args: unknown[]) => Promise<unknown>
    }
  }
}
