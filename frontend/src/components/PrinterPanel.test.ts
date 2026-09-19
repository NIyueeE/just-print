import { describe, expect, it } from 'vitest'
import type { OptionSpec, Printer } from '../api'
import { payloadOptions } from './PrinterPanel'

function makePrinter(options: Printer['options']): Printer {
  return {
    id: 'CUPS-PDF',
    name: 'CUPS-PDF',
    state: 'idle',
    accepting_jobs: true,
    make_and_model: null,
    location: null,
    options_error: null,
    options,
  }
}

const media: OptionSpec = {
  kind: 'keyword',
  default: 'iso_a4_210x297mm',
  values: ['iso_a4_210x297mm'],
}

describe('payloadOptions', () => {
  it('drops keys the printer no longer exposes', () => {
    const printer = makePrinter({ media })
    expect(
      payloadOptions(printer, { media: 'iso_a4_210x297mm', sides: 'two-sided-long-edge' }),
    ).toEqual({ media: 'iso_a4_210x297mm' })
  })

  it('drops cleared (empty) values so the printer default applies', () => {
    const printer = makePrinter({ media })
    expect(payloadOptions(printer, { media: '' })).toEqual({})
  })
})
