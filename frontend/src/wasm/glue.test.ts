// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

// The wasm-bindgen glue in ./generated is committed code the app ships, so it
// is held to the same coverage bar as hand-written code: its error paths and
// every way its loader can be handed a module are exercised here, each on a
// fresh copy of the glue so the module-level state starts uninitialised.
import { afterEach, describe, expect, it, vi } from 'vitest'

import type * as GlueModule from './generated/domain_wasm'
import { nextCronRuns, renderNotificationTemplate } from './generated/domain_wasm'
import { serializeFileChangePatterns } from './domain'

import wasmDataUrl from './generated/domain_wasm_bg.wasm?inline'

const wasmBytes = Uint8Array.from(atob(wasmDataUrl.slice(wasmDataUrl.indexOf(',') + 1)), (c) =>
  c.charCodeAt(0),
)

type Bindings = typeof GlueModule

async function freshBindings(): Promise<Bindings> {
  vi.resetModules()
  return import('./generated/domain_wasm')
}

function wasmResponse(body: BodyInit, init?: ResponseInit): Response {
  return new Response(body, { headers: { 'Content-Type': 'application/wasm' }, ...init })
}

// happy-dom's Response is not the engine's own, which V8's instantiateStreaming
// insists on; compile the body the way a browser's streaming compile would.
function streamLikeABrowser(): void {
  vi.spyOn(WebAssembly, 'instantiateStreaming').mockImplementation(async (source, imports) => {
    const response = await source
    if (response.headers.get('Content-Type') !== 'application/wasm') {
      throw new TypeError("Incorrect response MIME type. Expected 'application/wasm'.")
    }
    return WebAssembly.instantiate(await response.arrayBuffer(), imports)
  })
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('glue error paths', () => {
  it('rejects a column holding something other than strings', () => {
    expect(() => serializeFileChangePatterns([{ path: 1, action: 'warn' } as never])).toThrow(
      'array contains a value of the wrong type',
    )
  })

  it('surfaces a payload that is not JSON as an error', () => {
    expect(() => renderNotificationTemplate('{{host}}', 'not json')).toThrow()
  })

  it('treats an offset lookup that throws as an unknown zone', () => {
    const offsets = {
      offsetSecondsAt(): number {
        throw new Error('Intl unavailable')
      },
    }
    expect(() =>
      nextCronRuns('0 2 * * *', '2026-01-01T00:00:00Z', 'Lost/Zone', offsets, 1),
    ).toThrow('has no local time in timezone Lost/Zone')
  })
})

describe('glue loader', () => {
  it('streams a module served as application/wasm', async () => {
    streamLikeABrowser()
    const glue = await freshBindings()
    await glue.default({ module_or_path: wasmResponse(wasmBytes) })
    expect(glue.maxHookCommandTimeoutSeconds()).toBe(86_400)
  })

  it('falls back to buffering when the server sends the wrong MIME type', async () => {
    streamLikeABrowser()
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const glue = await freshBindings()
    const response = new Response(wasmBytes, { headers: { 'Content-Type': 'text/plain' } })
    await glue.default({ module_or_path: response })
    expect(warn).toHaveBeenCalledWith(
      expect.stringContaining('application/wasm'),
      expect.anything(),
    )
    expect(glue.validateCron('0 2 * * *')).toBeUndefined()
  })

  it('reports a failed fetch', async () => {
    const glue = await freshBindings()
    const response = wasmResponse('missing', { status: 404, statusText: 'Not Found' })
    await expect(glue.default({ module_or_path: response })).rejects.toThrow(
      'failed to fetch Wasm: 404 Not Found',
    )
  })

  it('rethrows a streaming failure that a correct MIME type cannot explain', async () => {
    streamLikeABrowser()
    const glue = await freshBindings()
    await expect(
      glue.default({ module_or_path: wasmResponse(new Uint8Array([0, 1, 2, 3])) }),
    ).rejects.toThrow(WebAssembly.CompileError)
  })

  it('rethrows a streaming failure on a response of an unexpected type', async () => {
    streamLikeABrowser()
    const glue = await freshBindings()
    const response = new Response(new Uint8Array([0, 1, 2, 3]))
    Object.defineProperty(response, 'type', { value: 'opaque' })
    await expect(glue.default({ module_or_path: response })).rejects.toThrow()
  })

  it('instantiates raw bytes', async () => {
    const glue = await freshBindings()
    await glue.default({ module_or_path: wasmBytes })
    expect(glue.parseFileChangePatterns('*/tmp* ignore')).toEqual([['*/tmp*', 'ignore']])
  })

  it('instantiates a compiled module', async () => {
    const glue = await freshBindings()
    await glue.default({ module_or_path: new WebAssembly.Module(wasmBytes) })
    expect(glue.notificationTemplatePlaceholderKeys()).toContain('host')
  })

  it('fetches a module given by URL', async () => {
    streamLikeABrowser()
    const fetch = vi.fn(async () => wasmResponse(wasmBytes))
    vi.stubGlobal('fetch', fetch)
    const glue = await freshBindings()
    await glue.default({ module_or_path: 'https://example.test/domain_wasm_bg.wasm' })
    expect(fetch).toHaveBeenCalledWith('https://example.test/domain_wasm_bg.wasm')
    expect(glue.validateCron('0 2 * * *')).toBeUndefined()
  })

  it('initialises only once', async () => {
    const glue = await freshBindings()
    const first = await glue.default({ module_or_path: wasmBytes })
    await expect(glue.default({ module_or_path: wasmBytes })).resolves.toBe(first)
  })

  it('warns about the deprecated positional initialisers', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const asyncGlue = await freshBindings()
    await asyncGlue.default(wasmBytes)
    const syncGlue = await freshBindings()
    syncGlue.initSync(wasmBytes)
    expect(warn).toHaveBeenCalledTimes(2)
    expect(syncGlue.maxHookCommandTimeoutSeconds()).toBe(86_400)
  })

  it('encodes strings on engines without TextEncoder.encodeInto', async () => {
    const encodeInto = Object.getOwnPropertyDescriptor(TextEncoder.prototype, 'encodeInto')
    Reflect.deleteProperty(TextEncoder.prototype, 'encodeInto')
    try {
      const glue = await freshBindings()
      glue.initSync({ module: wasmBytes })
      expect(glue.parseFileChangePatterns('*/Übersicht* fatal')).toEqual([
        ['*/Übersicht*', 'fatal'],
      ])
    } finally {
      if (encodeInto) Object.defineProperty(TextEncoder.prototype, 'encodeInto', encodeInto)
    }
  })
})
