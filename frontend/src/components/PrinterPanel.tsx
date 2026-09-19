import { useCallback, useEffect, useRef } from 'preact/hooks'
import {
  errorMessage,
  isAbortError,
  listPrinters,
  retryAfterHint,
  type OptionSpec,
  type Printer,
} from '../api'
import { useAuthGuard } from '../hooks/useAuthGuard'
import { usePolling } from '../hooks/usePolling'
import { AlertIcon, InfoIcon, PrinterIcon, RefreshIcon, SendIcon } from '../icons'
import {
  controlLabel,
  isNumericOption,
  optionChoices,
  optionUnit,
  sortOptionKeys,
  summarizeOptions,
} from '../options'
import { useAppDispatch, useAppState } from '../state'
import './PrinterPanel.css'

const PRINTER_STATE_LABELS: Record<Printer['state'], string> = {
  idle: '空闲',
  printing: '打印中',
  stopped: '已停止',
  disabled: '已禁用',
}

function OptionField({ optionKey, spec }: { optionKey: string; spec: OptionSpec }) {
  const state = useAppState()
  const dispatch = useAppDispatch()
  const unit = optionUnit(optionKey, spec)
  const value = state.options[optionKey] ?? ''

  return (
    <label class="field">
      <span class="field__label">
        {controlLabel(optionKey)}
        {unit !== null ? `（${unit}）` : ''}
      </span>
      {isNumericOption(spec) ? (
        <input
          type="number"
          inputMode="numeric"
          min={spec.min}
          max={spec.max}
          step={1}
          value={value}
          onInput={(event) => {
            // 空值也要写回状态：否则 DOM 已经清空、state 里仍是旧值，
            // 确认对话框与真正提交的份数会和用户看到的不一致（视为“打印机默认”）。
            dispatch({ type: 'options/set', key: optionKey, value: event.currentTarget.value })
          }}
        />
      ) : (
        <select
          value={value}
          onChange={(event) => {
            dispatch({ type: 'options/set', key: optionKey, value: event.currentTarget.value })
          }}
        >
          {optionChoices(optionKey, spec).map((choice) => (
            <option key={choice.value} value={choice.value}>
              {choice.label}
            </option>
          ))}
        </select>
      )}
    </label>
  )
}

/**
 * 构造提交给服务端的选项目录：
 *   - 丢掉空值：数字输入框被清空表示「使用打印机默认」，不能把空串发给 IPP 编码器；
 *   - 丢掉打印机当前目录里不存在的键：打印机重置或读取选项目录失败后，
 *     残留的旧键会被服务端判为 `invalid_controls`（400）而整单被拒。
 */
export function payloadOptions(
  printer: Printer,
  values: Record<string, string>,
): Record<string, string> {
  const result: Record<string, string> = {}
  for (const [key, value] of Object.entries(values)) {
    if (value !== '' && key in printer.options) {
      result[key] = value
    }
  }
  return result
}

export function PrinterPanel() {
  const state = useAppState()
  const dispatch = useAppDispatch()
  const authGuard = useAuthGuard()
  const { printers, selectedPrinterId, options, upload } = state
  const abortRef = useRef<AbortController | null>(null)

  // 组件卸载（例如退出登录）时中断手动刷新，避免响应落到已经重置的会话状态里。
  useEffect(() => {
    return () => {
      abortRef.current?.abort()
      abortRef.current = null
    }
  }, [])

  const load = useCallback(
    async (signal: AbortSignal, bypassCache: boolean): Promise<void> => {
      const items = await listPrinters({ refresh: bypassCache, signal })
      dispatch({ type: 'printers/loaded', printers: items })
    },
    [dispatch],
  )

  usePolling(
    async (signal) => {
      dispatch({ type: 'printers/loading' })
      try {
        await load(signal, false)
      } catch (error) {
        if (isAbortError(error)) {
          return
        }
        if (authGuard(error)) {
          return
        }
        dispatch({
          type: 'printers/error',
          message: `打印机列表加载失败：${errorMessage(error)}${retryAfterHint(error)}`,
        })
        throw error
      }
    },
    { intervalMs: 5000, enabled: state.token !== '' },
  )

  async function handleManualRefresh(): Promise<void> {
    if (printers.refreshing) {
      return
    }
    abortRef.current?.abort()
    const controller = new AbortController()
    abortRef.current = controller
    dispatch({ type: 'printers/refreshing', value: true })
    try {
      await load(controller.signal, true)
    } catch (error) {
      if (isAbortError(error) || controller.signal.aborted) {
        return
      }
      if (authGuard(error)) {
        return
      }
      dispatch({
        type: 'printers/error',
        message: `打印机列表加载失败：${errorMessage(error)}${retryAfterHint(error)}`,
      })
    } finally {
      // 只有当前请求才允许清掉 spinner：被后发请求取代的旧请求结束时不能提前收起。
      if (abortRef.current === controller) {
        abortRef.current = null
        dispatch({ type: 'printers/refreshing', value: false })
      }
    }
  }

  const selected = printers.items.find((printer) => printer.id === selectedPrinterId) ?? null
  const canPrint =
    upload.result !== null &&
    selected !== null &&
    selected.accepting_jobs &&
    !state.print.submitting

  function openConfirmation(): void {
    if (upload.result === null || selected === null) {
      return
    }
    const chosen = payloadOptions(selected, options)
    dispatch({
      type: 'print/confirm',
      pending: {
        payload: { file_id: upload.result.id, printer_id: selected.id, options: chosen },
        printerName: selected.name,
        fileName: upload.result.name,
        summary: summarizeOptions(selected, chosen).map((row) => ({
          label: row.label,
          value: row.value,
        })),
      },
    })
  }

  const optionKeys = selected !== null ? sortOptionKeys(Object.keys(selected.options)) : []

  return (
    <section class="card printer-panel" aria-labelledby="printer-panel-title">
      <div class="card__header">
        <span class="step-badge step-badge--print" aria-hidden="true">
          2
        </span>
        <span class="card__icon card__icon--print" aria-hidden="true">
          <PrinterIcon size={18} />
        </span>
        <h2 id="printer-panel-title">打印</h2>
        <button
          type="button"
          class="ghost ghost--sm"
          onClick={() => void handleManualRefresh()}
          disabled={printers.refreshing}
          title="刷新打印机列表（跳过服务端缓存）"
        >
          <RefreshIcon size={14} className={printers.refreshing ? 'refresh-spin' : undefined} />
          刷新
        </button>
      </div>

      <div class="printer-panel__body" aria-busy={printers.status === 'loading'}>
        {/* 刷新失败时如果已经拿到过打印机列表，就只在顶部提示错误：
            整块替换成错误页会让选择框、选项和「提交打印」一起消失，
            用户明明还能用却像服务挂了。 */}
        {printers.error !== null ? (
          <div class="inline-error" role="alert">
            <AlertIcon size={16} />
            <span>
              {printers.error}。请确认容器内 CUPS 服务已启动（可用 JUST_PRINT_CUPS_PDF=1 添加
              CUPS-PDF 调试打印机），服务会自动重试。
            </span>
          </div>
        ) : null}

        {/* 从未拿到过打印机列表、也没有错误时，才提示「未发现打印机」。 */}
        {printers.items.length === 0 && printers.error === null ? (
          <div class="inline-hint">
            <RefreshIcon size={15} className="refresh-spin" />
            <span>
              未发现打印机。请先在 CUPS 中配置打印机（容器内可用 JUST_PRINT_CUPS_PDF=1 添加 CUPS-PDF
              调试打印机），服务会自动重试。
            </span>
          </div>
        ) : null}

        {printers.items.length > 0 ? (
          <>
            <label class="field">
              <span class="field__label">打印机</span>
              <select
                value={selectedPrinterId}
                onChange={(event) =>
                  dispatch({ type: 'printer/select', id: event.currentTarget.value })
                }
              >
                {printers.items.map((printer) => (
                  <option key={printer.id} value={printer.id}>
                    {printer.name}
                    {!printer.accepting_jobs ? '（不接收任务）' : ''}
                  </option>
                ))}
              </select>
            </label>

            {selected !== null ? (
              <div class="printer-panel__summary">
                <span class={`printer-state printer-state--${selected.state}`}>
                  {PRINTER_STATE_LABELS[selected.state]}
                </span>
                {!selected.accepting_jobs ? (
                  <span class="printer-state printer-state--disabled">不接收任务</span>
                ) : null}
                {selected.make_and_model !== null ? (
                  <span class="printer-panel__detail">{selected.make_and_model}</span>
                ) : null}
                {selected.location !== null ? (
                  <span class="printer-panel__detail">位置：{selected.location}</span>
                ) : null}
                {optionKeys.length > 0 ? (
                  <span class="printer-panel__detail">{optionKeys.length} 个可调选项</span>
                ) : null}
              </div>
            ) : null}

            {selected?.options_error !== null && selected?.options_error !== undefined ? (
              <div class="inline-hint inline-hint--warning">
                <InfoIcon size={15} />
                <span>
                  读取该打印机的选项目录失败：{selected.options_error}，将使用默认设置打印。
                </span>
              </div>
            ) : null}

            {selected !== null && optionKeys.length === 0 ? (
              <div class="inline-hint">
                <InfoIcon size={15} />
                <span>该打印机没有可用的 CUPS 选项，将以默认设置打印。</span>
              </div>
            ) : null}

            {selected !== null && optionKeys.length > 0 ? (
              <div class="printer-panel__controls">
                {optionKeys.map((key) => (
                  <OptionField key={key} optionKey={key} spec={selected.options[key]} />
                ))}
              </div>
            ) : null}

            {selected !== null && optionKeys.length > 0 ? (
              <button
                type="button"
                class="ghost ghost--sm printer-panel__reset"
                onClick={() => dispatch({ type: 'options/reset' })}
              >
                <RefreshIcon size={14} />
                恢复推荐设置（A4 · 双面长边 · 最高分辨率）
              </button>
            ) : null}

            {upload.result === null ? (
              <div class="inline-hint">
                <InfoIcon size={15} />
                <span>请先上传文档后再打印。</span>
              </div>
            ) : null}

            {selected !== null && !selected.accepting_jobs ? (
              <div class="inline-error" role="alert">
                <AlertIcon size={16} />
                <span>该打印机当前不接收新任务，请在 CUPS 中恢复后再试。</span>
              </div>
            ) : null}

            <button
              type="button"
              class="primary printer-panel__submit"
              disabled={!canPrint}
              onClick={openConfirmation}
            >
              <SendIcon size={16} />
              提交打印
            </button>
          </>
        ) : null}
      </div>
    </section>
  )
}
