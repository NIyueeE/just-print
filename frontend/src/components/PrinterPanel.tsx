import { useEffect, useRef, useState } from 'preact/hooks'
import type { JSX } from 'preact'
import {
  type OptionView,
  type Printer,
  type UploadResult,
  errorMessage,
  isUnauthorized,
  listPrinters,
  submitPrint,
} from '../api'
import type { Notice } from '../app'
import {
  AlertIcon,
  InfoIcon,
  PrinterIcon,
  RefreshIcon,
  SendIcon,
  SpinnerIcon,
} from '../icons'

interface PrinterPanelProps {
  upload: UploadResult | null
  onJobSubmitted: (jobId: string, printerName: string) => void
  onAuthFailure: () => void
  onNotice: (notice: Notice | null) => void
}

/** 常用打印偏好：A4 纸张、最大分辨率、双面长边装订。 */
function preferredValue(key: string, option: OptionView): string | null {
  if (option.kind === 'enumerated') {
    const values = option.values ?? []
    const normalized = values.map((value) => value.toLowerCase())
    if (key === 'PageSize' || key === 'media') {
      const a4 = normalized.indexOf('a4')
      return a4 !== -1 ? values[a4] : null
    }
    if (key === 'Duplex' || key === 'sides') {
      const duplex = ['duplexnotumble', 'two-sided-long-edge', 'longedge', 'duplex']
        .map((value) => normalized.indexOf(value))
        .find((index) => index !== -1)
      return duplex !== undefined ? values[duplex] : null
    }
    if (/binding/i.test(key)) {
      const longEdge = normalized.indexOf('longedge')
      return longEdge !== -1 ? values[longEdge] : null
    }
    if (/resolution|dpi/i.test(key)) {
      let best: string | null = null
      let bestDpi = -1
      for (const value of values) {
        const dpi = Number.parseInt(value, 10)
        if (Number.isFinite(dpi) && dpi > bestDpi) {
          bestDpi = dpi
          best = value
        }
      }
      return best
    }
    return null
  }
  if (option.kind === 'range' && /resolution|dpi/i.test(key) && option.max !== undefined) {
    return String(option.max)
  }
  return null
}

function defaultsFor(printer: Printer): Record<string, string> {
  const result: Record<string, string> = {}
  for (const [key, option] of Object.entries(printer.options)) {
    const preferred = preferredValue(key, option)
    if (preferred !== null) {
      result[key] = preferred
    } else if (option.default !== null && option.default !== undefined) {
      result[key] = option.default
    } else if (option.kind === 'enumerated' && option.values && option.values.length > 0) {
      result[key] = option.values[0]
    } else if (option.kind === 'range' && option.min !== undefined) {
      result[key] = String(option.min)
    }
  }
  return result
}

const CONTROL_LABELS: Record<string, string> = {
  PageSize: '纸张大小',
  media: '纸张',
  Duplex: '双面打印',
  sides: '单双面',
  ColorModel: '颜色',
  cupsPrintQuality: '打印质量',
  PrintQuality: '打印质量',
  Resolution: '分辨率',
  copies: '份数',
  NumberUp: '每页版面',
  Collate: '逐份打印',
  InputSlot: '进纸盒',
  MediaType: '纸张类型',
  OutputMode: '输出模式',
}

const VALUE_LABELS: Record<string, Record<string, string>> = {
  Duplex: {
    None: '单面',
    DuplexNoTumble: '双面（长边装订）',
    DuplexTumble: '双面（短边装订）',
  },
  sides: {
    'one-sided': '单面',
    'two-sided-long-edge': '双面（长边装订）',
    'two-sided-short-edge': '双面（短边装订）',
  },
  ColorModel: { RGB: '彩色', Gray: '灰度' },
  cupsPrintQuality: { Draft: '草稿', Normal: '普通', High: '高' },
  PrintQuality: { Draft: '草稿', Normal: '普通', High: '高' },
  MediaType: { Plain: '普通纸', PhotoPaper: '相纸', Envelope: '信封' },
  Collate: { True: '逐份', False: '不逐份' },
  OutputMode: { Print: '直接打印', Preview: '预览' },
}

const UNIT_LABELS: Record<string, string> = {
  Resolution: 'dpi',
  copies: '份',
  NumberUp: '版',
}

const STATE_LABELS: Record<string, string> = {
  idle: '空闲',
  printing: '打印中',
  disabled: '已禁用',
  stopped: '已停止',
}

export function PrinterPanel({
  upload,
  onJobSubmitted,
  onAuthFailure,
  onNotice,
}: PrinterPanelProps) {
  const [printers, setPrinters] = useState<Printer[]>([])
  const [selectedId, setSelectedId] = useState('')
  const [controls, setControls] = useState<Record<string, string>>({})
  const [busy, setBusy] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const mountedRef = useRef(true)
  const refreshingRef = useRef(false)
  const selectedIdRef = useRef('')

  useEffect(() => {
    mountedRef.current = true
    void refresh()
    const interval = window.setInterval(() => {
      if (!document.hidden) {
        void refresh()
      }
    }, 5000)
    return () => {
      mountedRef.current = false
      window.clearInterval(interval)
    }
  }, [])

  async function refresh(): Promise<void> {
    if (refreshingRef.current) {
      return
    }
    refreshingRef.current = true
    setRefreshing(true)
    try {
      const result = await listPrinters()
      if (!mountedRef.current) {
        return
      }
      setPrinters(result.printers)
      setError(null)
      const previous = selectedIdRef.current
      const keep = previous && result.printers.some((printer) => printer.id === previous)
      const next = keep ? previous : (result.printers[0]?.id ?? '')
      selectedIdRef.current = next
      setSelectedId(next)
      if (!keep) {
        const printer = result.printers.find((candidate) => candidate.id === next)
        setControls(printer ? defaultsFor(printer) : {})
      }
    } catch (requestError) {
      if (isUnauthorized(requestError)) {
        onAuthFailure()
        return
      }
      if (mountedRef.current) {
        setError(`打印机列表加载失败：${errorMessage(requestError)}`)
      }
    } finally {
      refreshingRef.current = false
      if (mountedRef.current) {
        setRefreshing(false)
      }
    }
  }

  function selectPrinter(id: string): void {
    selectedIdRef.current = id
    setSelectedId(id)
    const printer = printers.find((candidate) => candidate.id === id)
    setControls(printer ? defaultsFor(printer) : {})
  }

  function setControl(key: string, value: string): void {
    setControls((previous) => ({ ...previous, [key]: value }))
  }

  function renderControl(key: string, option: OptionView): JSX.Element | null {
    if (option.kind === 'enumerated') {
      const options = option.values ?? []
      if (options.length === 0) {
        return null
      }
      return (
        <select
          aria-label={CONTROL_LABELS[key] ?? key}
          value={controls[key] ?? ''}
          onInput={(event) =>
            setControl(key, (event.target as HTMLSelectElement).value)}
        >
          {options.map((value) => (
            <option key={value} value={value}>
              {VALUE_LABELS[key]?.[value] ?? value}
            </option>
          ))}
        </select>
      )
    }
    const min = option.min ?? 0
    const max = option.max ?? 100
    return (
      <input
        type="number"
        inputMode="numeric"
        min={min}
        max={max}
        aria-label={CONTROL_LABELS[key] ?? key}
        value={controls[key] ?? ''}
        onInput={(event) =>
          setControl(key, (event.target as HTMLInputElement).value)}
      />
    )
  }

  const selected = printers.find((printer) => printer.id === selectedId) ?? null
  const canPrint = upload !== null && selected !== null && !busy

  async function handlePrint(): Promise<void> {
    if (!upload || !selected || busy) {
      return
    }
    setBusy(true)
    setError(null)
    onNotice(null)
    try {
      const result = await submitPrint({
        file_id: upload.id,
        printer_id: selected.id,
        options: controls,
      })
      onJobSubmitted(result.job_id, selected.name)
      onNotice({ message: '打印任务已提交给 CUPS', kind: 'success' })
    } catch (requestError) {
      if (isUnauthorized(requestError)) {
        onAuthFailure()
        return
      }
      setError(`提交失败：${errorMessage(requestError)}`)
    } finally {
      setBusy(false)
    }
  }

  return (
    <section class="card card-print">
      <div class="card-header">
        <span class="step-badge">2</span>
        <span class="card-icon">
          <PrinterIcon size={18} />
        </span>
        <h2>打印</h2>
        <button
          type="button"
          class="ghost ghost-sm"
          onClick={() => void refresh()}
          disabled={refreshing}
          title="刷新打印机列表"
        >
          <RefreshIcon size={14} className={refreshing ? 'refresh-spin' : undefined} />
          刷新
        </button>
      </div>
      {error ? (
        <div class="error-box" role="alert">
          <AlertIcon size={16} />
          <span>
            {error}，请确认容器内 CUPS 服务已启动（可用{' '}
            <code>JUST_PRINT_CUPS_PDF=1</code> 添加 CUPS-PDF 调试打印机），
            服务每 5 秒自动重试。
          </span>
        </div>
      ) : printers.length === 0 ? (
        <div class="hint">
          <RefreshIcon size={15} className="refresh-spin" />
          <span>未发现打印机。请先在 CUPS 中配置打印机（容器内可用 JUST_PRINT_CUPS_PDF=1 添加 CUPS-PDF 调试打印机），服务会每 5 秒自动重试。</span>
        </div>
      ) : (
        <>
          <label class="field">
            <span class="field-label">打印机</span>
            <select
              value={selectedId}
              onInput={(event) => selectPrinter((event.target as HTMLSelectElement).value)}
            >
              {printers.map((printer) => (
                <option key={printer.id} value={printer.id}>
                  {printer.name}
                </option>
              ))}
            </select>
          </label>
          {selected ? (
            <div class="printer-summary">
              {selected.state ? (
                <span class={`lang-badge state-${selected.state}`}>
                  {STATE_LABELS[selected.state] ?? selected.state}
                </span>
              ) : null}
              {Object.keys(selected.options).length > 0 ? (
                <span class="printer-detail">
                  {Object.keys(selected.options).length} 个可调选项
                </span>
              ) : null}
            </div>
          ) : null}
          {selected && Object.keys(selected.options).length === 0 ? (
            <div class="hint">
              <InfoIcon size={15} />
              <span>该打印机没有可用的 CUPS 选项，将以默认设置打印。</span>
            </div>
          ) : null}
          {selected && Object.keys(selected.options).length > 0 ? (
            <div class="controls">
              {Object.entries(selected.options).map(([key, option]) => (
                <label class="field" key={key}>
                  <span class="field-label">
                    <span>
                      {CONTROL_LABELS[key] ?? key}
                      {option.kind === 'range' && UNIT_LABELS[key]
                        ? `（${UNIT_LABELS[key]}）`
                        : ''}
                    </span>
                  </span>
                  {renderControl(key, option)}
                </label>
              ))}
            </div>
          ) : null}
          {!upload ? (
            <div class="hint">
              <InfoIcon size={15} />
              <span>请先上传文档后再打印。</span>
            </div>
          ) : null}
          <button
            type="button"
            class="primary"
            disabled={!canPrint}
            onClick={() => void handlePrint()}
          >
            {busy ? (
              <>
                <SpinnerIcon size={16} />
                提交中…
              </>
            ) : (
              <>
                <SendIcon size={16} />
                提交打印
              </>
            )}
          </button>
        </>
      )}
    </section>
  )
}
