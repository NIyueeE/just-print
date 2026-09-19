/* =============================================================================
 * 打印选项的纯逻辑：默认值推导、控件选择项、中文标签与摘要。
 *
 * 产品预设（有意保留，不改为“留空交给 CUPS”）：
 *   - `media` 优先 A4（值中含 `a4`，如 iso_a4_210x297mm）；
 *   - `sides` 优先双面长边（two-sided-long-edge）；
 *   - `printer-resolution` 优先最高 DPI。
 * 其余选项回退到服务端 default，再回退到第一个合法值。
 * ========================================================================== */

import type { EnumOptionValue, OptionSpec, Printer, ResolutionValue } from './api'

/** IPP 选项展示顺序；未列出的键排在其后。 */
const OPTION_ORDER: readonly string[] = [
  'media',
  'sides',
  'print-color-mode',
  'print-quality',
  'printer-resolution',
  'copies',
  'number-up',
  'media-type',
  'output-bin',
  'orientation-requested',
  'finishings',
]

const CONTROL_LABELS: Record<string, string> = {
  media: '纸张大小',
  sides: '单双面',
  'print-color-mode': '色彩模式',
  'print-quality': '打印质量',
  'printer-resolution': '分辨率',
  copies: '份数',
  'number-up': '每张版面',
  'media-type': '纸张类型',
  'output-bin': '输出纸盒',
  'orientation-requested': '页面方向',
  finishings: '装订方式',
}

const VALUE_LABELS: Record<string, Record<string, string>> = {
  sides: {
    'one-sided': '单面',
    'two-sided-long-edge': '双面（长边装订）',
    'two-sided-short-edge': '双面（短边装订）',
  },
  'print-color-mode': {
    color: '彩色',
    monochrome: '黑白',
    auto: '自动',
    'auto-monochrome': '自动（黑白）',
    'process-monochrome': '处理后黑白',
  },
  'print-quality': {
    draft: '草稿',
    normal: '普通',
    high: '高',
  },
  'orientation-requested': {
    portrait: '纵向',
    landscape: '横向',
    'reverse-landscape': '横向（反向）',
    'reverse-portrait': '纵向（反向）',
  },
  finishings: {
    none: '无',
    staple: '装订',
    punch: '打孔',
    cover: '封面',
    bind: '胶装',
    'staple-top-left': '左上装订',
    'staple-bottom-left': '左下装订',
  },
  'media-type': {
    stationary: '普通纸',
    stationery: '普通纸',
    'stationery-letterhead': '信纸',
    photographic: '相纸',
    envelope: '信封',
    transparency: '透明胶片',
    labels: '标签纸',
  },
  'output-bin': {
    'face-down': '正面朝下',
    'face-up': '正面朝上',
  },
}

const UNIT_LABELS: Record<string, string> = {
  copies: '份',
  'number-up': '版',
  'printer-resolution': 'dpi',
}

const A4_PATTERN = /(^|[^a-z0-9])a4([^0-9]|$)/i
const DUPLEX_LONG_EDGE = new Set(['twosidedlongedge'])

function normalizeToken(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]/g, '')
}

/** 判断选项值是否表示 A4 纸张。 */
export function isA4Value(value: string): boolean {
  return A4_PATTERN.test(value)
}

/** 判断选项值是否表示“双面长边装订”。 */
export function isDuplexLongEdgeValue(value: string): boolean {
  return DUPLEX_LONG_EDGE.has(normalizeToken(value))
}

function isResolutionKey(key: string): boolean {
  return /resolution|dpi/i.test(key)
}

function resolutionScore(value: string): number {
  const match = /(\d{2,5})/.exec(value)
  return match ? Number(match[1]) : -1
}

function highestByDpi(values: readonly string[]): string | null {
  let best: string | null = null
  let bestScore = -1
  for (const value of values) {
    const score = resolutionScore(value)
    if (score > bestScore) {
      bestScore = score
      best = value
    }
  }
  return bestScore > 0 ? best : null
}

function highestResolution(values: readonly ResolutionValue[]): ResolutionValue | null {
  let best: ResolutionValue | null = null
  let bestScore = -1
  for (const value of values) {
    const score = value.cross_feed * value.feed
    if (score > bestScore) {
      bestScore = score
      best = value
    }
  }
  return best
}

function preferredKeyword(key: string, values: readonly string[]): string | null {
  if (key === 'media') {
    return values.find(isA4Value) ?? null
  }
  if (key === 'sides') {
    return values.find(isDuplexLongEdgeValue) ?? null
  }
  if (isResolutionKey(key)) {
    return highestByDpi(values)
  }
  return null
}

function preferredEnum(key: string, values: readonly EnumOptionValue[]): string | null {
  if (key === 'media') {
    return values.find((value) => isA4Value(value.name))?.name ?? null
  }
  if (key === 'sides') {
    return values.find((value) => isDuplexLongEdgeValue(value.name))?.name ?? null
  }
  if (isResolutionKey(key)) {
    let best: EnumOptionValue | null = null
    for (const value of values) {
      if (best === null || value.value > best.value) {
        best = value
      }
    }
    return best?.name ?? null
  }
  return null
}

/** 单个选项的默认提交值；无法确定时返回 null。 */
export function defaultOptionValue(key: string, spec: OptionSpec): string | null {
  switch (spec.kind) {
    case 'keyword':
      return preferredKeyword(key, spec.values) ?? spec.default ?? spec.values[0] ?? null
    case 'enum':
      return preferredEnum(key, spec.values) ?? spec.default ?? spec.values[0]?.name ?? null
    case 'integer':
      if (isResolutionKey(key)) {
        return String(spec.max)
      }
      return spec.default ?? String(spec.min)
    case 'integer_choices': {
      if (spec.values.length === 0) {
        return spec.default
      }
      if (isResolutionKey(key)) {
        return String(Math.max(...spec.values))
      }
      return spec.default ?? String(spec.values[0])
    }
    case 'resolution': {
      const best = highestResolution(spec.values)
      return best?.label ?? spec.default ?? null
    }
  }
}

/** 为一台打印机推导全部默认选项。 */
export function deriveDefaults(printer: Printer): Record<string, string> {
  const result: Record<string, string> = {}
  for (const [key, spec] of Object.entries(printer.options)) {
    const value = defaultOptionValue(key, spec)
    if (value !== null) {
      result[key] = value
    }
  }
  return result
}

export function controlLabel(key: string): string {
  return CONTROL_LABELS[key] ?? key
}

function valueLabel(key: string, value: string): string {
  return VALUE_LABELS[key]?.[value] ?? value
}

function unitLabel(key: string, spec: OptionSpec): string | null {
  if (spec.kind === 'resolution') {
    return null
  }
  return UNIT_LABELS[key] ?? null
}

export interface OptionChoice {
  value: string
  label: string
}

/** 生成可选项；数值型（integer）返回空数组，由数字输入框处理。 */
export function optionChoices(key: string, spec: OptionSpec): OptionChoice[] {
  switch (spec.kind) {
    case 'keyword':
      return spec.values.map((value) => ({ value, label: valueLabel(key, value) }))
    case 'enum':
      return spec.values.map((value) => ({ value: value.name, label: valueLabel(key, value.name) }))
    case 'integer_choices':
      return spec.values.map((value) => ({
        value: String(value),
        label: `${value}${UNIT_LABELS[key] ?? ''}`,
      }))
    case 'resolution':
      return spec.values.map((value) => ({ value: value.label, label: value.label }))
    case 'integer':
      return []
  }
}

export function optionUnit(key: string, spec: OptionSpec): string | null {
  return unitLabel(key, spec)
}

export function isNumericOption(
  spec: OptionSpec,
): spec is Extract<OptionSpec, { kind: 'integer' }> {
  return spec.kind === 'integer'
}

/** 把提交值翻译为可读文案，用于确认对话框与任务详情。 */
export function describeOptionValue(key: string, spec: OptionSpec, value: string): string {
  switch (spec.kind) {
    case 'keyword':
      return valueLabel(key, value)
    case 'enum': {
      const found = spec.values.find(
        (candidate) => candidate.name === value || String(candidate.value) === value,
      )
      return found ? valueLabel(key, found.name) : value
    }
    case 'integer':
      return `${value}${UNIT_LABELS[key] ?? ''}`
    case 'integer_choices':
      return `${value}${UNIT_LABELS[key] ?? ''}`
    case 'resolution': {
      const found = spec.values.find((candidate) => candidate.label === value)
      return found?.label ?? value
    }
  }
}

/** 按展示顺序排列选项键。 */
export function sortOptionKeys(keys: readonly string[]): string[] {
  const rank = new Map(OPTION_ORDER.map((key, index) => [key, index]))
  return [...keys].sort((a, b) => {
    const rankA = rank.get(a) ?? Number.MAX_SAFE_INTEGER
    const rankB = rank.get(b) ?? Number.MAX_SAFE_INTEGER
    if (rankA !== rankB) {
      return rankA - rankB
    }
    return a.localeCompare(b)
  })
}

export interface OptionSummaryRow {
  key: string
  label: string
  value: string
}

/** 确认对话框必须覆盖的关键项（即使打印机未暴露该选项也要说明“打印机默认”）。 */
const SUMMARY_KEYS: readonly string[] = ['copies', 'sides', 'media', 'printer-resolution']

/**
 * 为确认对话框生成选项摘要：
 *   - 始终包含份数 / 单双面 / 纸张 / 分辨率，缺失时显示“打印机默认”；
 *   - 其余已设置且有目录定义的选项一并展示。
 */
export function summarizeOptions(
  printer: Printer,
  options: Record<string, string>,
): OptionSummaryRow[] {
  const keys = new Set<string>(Object.keys(options).filter((key) => key in printer.options))
  for (const key of SUMMARY_KEYS) {
    if (key in printer.options) {
      keys.add(key)
    }
  }
  return sortOptionKeys([...keys]).map((key) => {
    const spec = printer.options[key]
    const value = options[key]
    return {
      key,
      label: controlLabel(key),
      value: value === undefined ? '打印机默认' : describeOptionValue(key, spec, value),
    }
  })
}
