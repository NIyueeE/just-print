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
  onJobSubmitted: (jobId: string) => void
  onAuthFailure: () => void
  onNotice: (message: string | null) => void
}

function defaultsFor(printer: Printer): Record<string, string> {
  const result: Record<string, string> = {}
  for (const [key, option] of Object.entries(printer.options)) {
    if (option.default !== null && option.default !== undefined) {
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
  const [error, setError] = useState<string | null>(null)
  const mountedRef = useRef(true)
  const selectedIdRef = useRef('')

  useEffect(() => {
    mountedRef.current = true
    void refresh()
    const interval = window.setInterval(() => void refresh(), 5000)
    return () => {
      mountedRef.current = false
      window.clearInterval(interval)
    }
  }, [])

  async function refresh(): Promise<void> {
    try {
      const result = await listPrinters()
      if (!mountedRef.current) {
        return
      }
      setPrinters(result.printers)
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
      onJobSubmitted(result.job_id)
      onNotice('打印任务已提交给 CUPS')
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
      </div>
      {printers.length === 0 ? (
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
      {error ? (
        <p class="error">
          <AlertIcon size={15} />
          {error}
        </p>
      ) : null}
    </section>
  )
}
