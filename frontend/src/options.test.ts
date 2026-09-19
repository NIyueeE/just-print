import { describe, expect, it } from 'vitest'
import type { Printer } from './api'
import {
  defaultOptionValue,
  deriveDefaults,
  describeOptionValue,
  isA4Value,
  isDuplexLongEdgeValue,
  optionChoices,
  summarizeOptions,
} from './options'

function makePrinter(options: Printer['options']): Printer {
  return {
    id: 'CUPS-PDF',
    name: 'CUPS-PDF Printer',
    state: 'idle',
    accepting_jobs: true,
    make_and_model: 'CUPS-PDF',
    location: null,
    options_error: null,
    options,
  }
}

describe('A4 / 双面长边 识别', () => {
  it('matches A4 keywords but not similar tokens', () => {
    expect(isA4Value('a4')).toBe(true)
    expect(isA4Value('A4')).toBe(true)
    expect(isA4Value('iso_a4_210x297mm')).toBe(true)
    expect(isA4Value('na_letter_8.5x11in')).toBe(false)
    expect(isA4Value('a400')).toBe(false)
  })

  it('matches the IPP duplex long-edge keyword', () => {
    expect(isDuplexLongEdgeValue('two-sided-long-edge')).toBe(true)
    expect(isDuplexLongEdgeValue('two_sided_long_edge')).toBe(true)
    expect(isDuplexLongEdgeValue('two-sided-short-edge')).toBe(false)
    expect(isDuplexLongEdgeValue('one-sided')).toBe(false)
  })
})

describe('默认选项推导', () => {
  it('prefers A4, duplex long edge and the highest resolution', () => {
    const printer = makePrinter({
      media: {
        kind: 'keyword',
        default: 'na_letter_8.5x11in',
        values: ['na_letter_8.5x11in', 'iso_a4_210x297mm'],
      },
      sides: {
        kind: 'keyword',
        default: 'one-sided',
        values: ['one-sided', 'two-sided-long-edge', 'two-sided-short-edge'],
      },
      'printer-resolution': {
        kind: 'resolution',
        default: '300x300dpi',
        values: [
          { cross_feed: 300, feed: 300, units: 3, label: '300x300dpi' },
          { cross_feed: 600, feed: 600, units: 3, label: '600x600dpi' },
          { cross_feed: 1200, feed: 1200, units: 3, label: '1200x1200dpi' },
        ],
      },
      copies: { kind: 'integer', default: '1', min: 1, max: 999 },
      'number-up': { kind: 'integer_choices', default: '1', values: [1, 2, 4, 6] },
      'print-quality': {
        kind: 'enum',
        default: 'normal',
        values: [
          { value: 3, name: 'draft' },
          { value: 4, name: 'normal' },
          { value: 5, name: 'high' },
        ],
      },
    })

    expect(deriveDefaults(printer)).toEqual({
      media: 'iso_a4_210x297mm',
      sides: 'two-sided-long-edge',
      'printer-resolution': '1200x1200dpi',
      copies: '1',
      'number-up': '1',
      'print-quality': 'normal',
    })
  })

  it('falls back to the server default when no preference matches', () => {
    const printer = makePrinter({
      media: {
        kind: 'keyword',
        default: 'na_letter_8.5x11in',
        values: ['na_letter_8.5x11in', 'na_legal_8.5x14in'],
      },
    })
    expect(deriveDefaults(printer)).toEqual({ media: 'na_letter_8.5x11in' })
  })

  it('picks the largest keyword resolution and the largest enum resolution', () => {
    expect(
      defaultOptionValue('printer-resolution', {
        kind: 'keyword',
        default: '300dpi',
        values: ['300dpi', '600dpi', '1200dpi'],
      }),
    ).toBe('1200dpi')

    expect(
      defaultOptionValue('printer-resolution', {
        kind: 'enum',
        default: 'draft',
        values: [
          { value: 150, name: 'draft' },
          { value: 1200, name: 'high' },
          { value: 600, name: 'normal' },
        ],
      }),
    ).toBe('high')
  })

  it('uses min for integer without default and max for resolution integers', () => {
    expect(defaultOptionValue('copies', { kind: 'integer', default: null, min: 1, max: 99 })).toBe(
      '1',
    )
    expect(
      defaultOptionValue('printer-resolution', {
        kind: 'integer',
        default: null,
        min: 150,
        max: 1200,
      }),
    ).toBe('1200')
  })

  it('keeps A4 keyword values in option choices and translates labels', () => {
    const choices = optionChoices('sides', {
      kind: 'keyword',
      default: 'one-sided',
      values: ['one-sided', 'two-sided-long-edge'],
    })
    expect(choices).toEqual([
      { value: 'one-sided', label: '单面' },
      { value: 'two-sided-long-edge', label: '双面（长边装订）' },
    ])

    expect(
      describeOptionValue(
        'printer-resolution',
        {
          kind: 'resolution',
          default: null,
          values: [{ cross_feed: 600, feed: 600, units: 3, label: '600x600dpi' }],
        },
        '600x600dpi',
      ),
    ).toBe('600x600dpi')
  })

  it('always summarizes copies, sides, media and resolution for confirmation', () => {
    const printer = makePrinter({
      media: { kind: 'keyword', default: null, values: ['iso_a4_210x297mm'] },
      sides: { kind: 'keyword', default: null, values: ['two-sided-long-edge'] },
      copies: { kind: 'integer', default: '1', min: 1, max: 99 },
      'printer-resolution': {
        kind: 'resolution',
        default: null,
        values: [{ cross_feed: 1200, feed: 1200, units: 3, label: '1200x1200dpi' }],
      },
    })
    const rows = summarizeOptions(printer, { media: 'iso_a4_210x297mm' })
    const byLabel = new Map(rows.map((row) => [row.label, row.value]))
    expect(byLabel.get('纸张大小')).toBe('iso_a4_210x297mm')
    expect(byLabel.get('单双面')).toBe('打印机默认')
    expect(byLabel.get('份数')).toBe('打印机默认')
    expect(byLabel.get('分辨率')).toBe('打印机默认')
  })

  it('treats a cleared numeric value as "printer default"', () => {
    const printer = makePrinter({
      copies: { kind: 'integer', default: '1', min: 1, max: 99 },
    })
    const rows = summarizeOptions(printer, { copies: '' })
    expect(rows).toEqual([{ key: 'copies', label: '份数', value: '打印机默认' }])
  })

  it('normalizes an enum default that is not a selectable choice', () => {
    // 服务端若给出枚举的数值形式（IPP enum），直接采用会让 <select> 匹配不到任何
    // option 而渲染成空白；这里必须回退到候选项里的等价名称。
    expect(
      defaultOptionValue('print-quality', {
        kind: 'enum',
        default: '4',
        values: [
          { value: 3, name: 'draft' },
          { value: 4, name: 'normal' },
          { value: 5, name: 'high' },
        ],
      }),
    ).toBe('normal')

    // 既不是名称也不是数值时回退到第一个候选项，保证控件始终有值。
    expect(
      defaultOptionValue('print-quality', {
        kind: 'enum',
        default: '9',
        values: [
          { value: 3, name: 'draft' },
          { value: 5, name: 'high' },
        ],
      }),
    ).toBe('draft')
  })

  it('normalizes resolution and integer_choices defaults outside their candidate list', () => {
    expect(
      defaultOptionValue('printer-resolution', {
        kind: 'resolution',
        default: '9999x9999dpi',
        values: [{ cross_feed: 600, feed: 600, units: 3, label: '600x600dpi' }],
      }),
    ).toBe('600x600dpi')

    expect(
      defaultOptionValue('number-up', { kind: 'integer_choices', default: '8', values: [1, 2, 4] }),
    ).toBe('1')
  })
})
