import { describe, expect, it } from 'vitest'
import { printFingerprint, resolveAttempt, type PrintAttempt } from './idempotency'

const payload = {
  file_id: 'file-1',
  printer_id: 'CUPS-PDF',
  options: { sides: 'two-sided-long-edge', media: 'iso_a4_210x297mm' },
}

describe('打印幂等键', () => {
  it('reuses the same key while retrying the same payload', () => {
    const fingerprint = printFingerprint(payload)
    const first = resolveAttempt(null, fingerprint, () => 'key-1')
    expect(first.reused).toBe(false)
    expect(first.attempt.key).toBe('key-1')

    const retry = resolveAttempt(first.attempt, fingerprint, () => 'key-2')
    expect(retry.reused).toBe(true)
    expect(retry.attempt.key).toBe('key-1')
  })

  it('creates a new key when the payload changes', () => {
    const first = resolveAttempt(null, printFingerprint(payload), () => 'key-1')
    const changed = { ...payload, options: { ...payload.options, copies: '2' } }
    const next = resolveAttempt(first.attempt, printFingerprint(changed), () => 'key-2')
    expect(next.reused).toBe(false)
    expect(next.attempt.key).toBe('key-2')
  })

  it('creates a new key after a confirmed success', () => {
    const fingerprint = printFingerprint(payload)
    const first = resolveAttempt(null, fingerprint, () => 'key-1')
    const settled: PrintAttempt = { ...first.attempt, settled: true }
    const next = resolveAttempt(settled, fingerprint, () => 'key-2')
    expect(next.reused).toBe(false)
    expect(next.attempt.key).toBe('key-2')
  })

  it('fingerprints option order-insensitively', () => {
    const reordered = {
      ...payload,
      options: { media: 'iso_a4_210x297mm', sides: 'two-sided-long-edge' },
    }
    expect(printFingerprint(reordered)).toBe(printFingerprint(payload))
  })
})
