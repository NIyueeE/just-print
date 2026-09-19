import { useEffect, useRef, useState } from 'preact/hooks'
import {
  ApiError,
  deleteFile,
  errorMessage,
  fetchPreview,
  getFormats,
  isAbortError,
  uploadFile,
  type Formats,
} from '../api'
import { formatBytes } from '../format'
import { useAuthGuard } from '../hooks/useAuthGuard'
import {
  AlertIcon,
  BanIcon,
  CheckIcon,
  ChevronDownIcon,
  ChevronUpIcon,
  EyeIcon,
  FileTextIcon,
  RefreshIcon,
  SpinnerIcon,
  UploadIcon,
  XIcon,
} from '../icons'
import { createNotice, useAppDispatch, useAppState } from '../state'
import './Uploader.css'

type FileCategory = 'pdf' | 'office' | 'image' | 'text' | 'other'

const OFFICE_EXTS = new Set([
  'doc',
  'docm',
  'docx',
  'dot',
  'dotm',
  'dotx',
  'xls',
  'xlsb',
  'xlsm',
  'xlsx',
  'xlt',
  'xltm',
  'xltx',
  'xlw',
  'ppt',
  'pptm',
  'pptx',
  'pot',
  'potm',
  'potx',
  'pps',
  'ppsx',
  'odt',
  'ods',
  'odp',
  'odg',
  'rtf',
  'csv',
  'tsv',
  'pages',
  'numbers',
  'key',
  'pub',
  'wps',
  'wpd',
  'et',
  'ett',
  'dps',
  'dpt',
])
const IMAGE_EXTS = new Set([
  'png',
  'jpg',
  'jpeg',
  'jpe',
  'jfif',
  'gif',
  'webp',
  'bmp',
  'tif',
  'tiff',
  'svg',
  'svgz',
  'psd',
  'eps',
  'emf',
  'wmf',
  'xbm',
  'pbm',
  'pgm',
  'ppm',
])
const TEXT_EXTS = new Set(['txt', 'md', 'htm', 'html', 'xhtml', 'xml', 'log'])

const CATEGORY_LABEL: Record<FileCategory, string> = {
  pdf: 'PDF',
  office: 'Office 文档',
  image: '图片',
  text: '文本',
  other: '文件',
}

function fileExtension(name: string): string {
  const index = name.lastIndexOf('.')
  return index >= 0 ? name.slice(index + 1).toLowerCase() : ''
}

function fileCategory(name: string): FileCategory {
  const ext = fileExtension(name)
  if (ext === 'pdf') return 'pdf'
  if (OFFICE_EXTS.has(ext)) return 'office'
  if (IMAGE_EXTS.has(ext)) return 'image'
  if (TEXT_EXTS.has(ext)) return 'text'
  return 'other'
}

/** 客户端预校验：大小上限与扩展名都来自 `GET /api/formats`。 */
export function validateFile(file: File, formats: Formats | null): string | null {
  if (formats === null) {
    return null
  }
  if (formats.max_upload_bytes > 0 && file.size > formats.max_upload_bytes) {
    return `文件大小 ${formatBytes(file.size)} 超过上限 ${formatBytes(formats.max_upload_bytes)}。`
  }
  const ext = fileExtension(file.name)
  if (ext.length === 0) {
    return '无法识别文件扩展名，请选择带有扩展名的文件。'
  }
  const supported = formats.extensions.some((candidate) => candidate.toLowerCase() === ext)
  if (!supported) {
    return `不支持 .${ext} 格式，请查看下方支持格式说明。`
  }
  return null
}

export function friendlyUploadError(error: unknown): string {
  if (error instanceof ApiError) {
    switch (error.code) {
      case 'payload_too_large':
        return '文件超过服务器允许的大小上限。'
      case 'unsupported_media_type':
        return '服务器不支持该文件格式。'
      case 'conversion_failed':
        return '文档转换失败，请确认文件可以正常打开。'
      case 'service_unavailable':
        return error.retryAfterMs !== null
          ? `服务繁忙，请约 ${Math.ceil(error.retryAfterMs / 1000)} 秒后重试。`
          : '服务繁忙，请稍后重试。'
      case 'timeout':
        return '上传超时，请重试。'
      case 'network':
        return '网络异常，请检查连接后重试。'
      default:
        return error.message
    }
  }
  return errorMessage(error)
}

export function Uploader() {
  const state = useAppState()
  const dispatch = useAppDispatch()
  const authGuard = useAuthGuard()
  const { upload, formats, formatsError } = state
  const [dragging, setDragging] = useState(false)
  const [previewOpen, setPreviewOpen] = useState(true)
  const inputRef = useRef<HTMLInputElement>(null)
  const uploadControllerRef = useRef<AbortController | null>(null)
  const previewControllerRef = useRef<AbortController | null>(null)

  const busy = upload.phase === 'uploading' || upload.phase === 'converting'
  const accept = formats
    ? formats.extensions.map((extension) => `.${extension}`).join(',')
    : undefined

  // 预览 blob URL 的回收：URL 变化或组件卸载时释放旧地址。
  useEffect(() => {
    const url = upload.previewUrl
    return () => {
      if (url !== null) {
        URL.revokeObjectURL(url)
      }
    }
  }, [upload.previewUrl])

  useEffect(() => {
    return () => {
      uploadControllerRef.current?.abort()
      previewControllerRef.current?.abort()
    }
  }, [])

  function openPicker(): void {
    if (!busy) {
      inputRef.current?.click()
    }
  }

  function selectFile(file: File | null): void {
    if (file === null) {
      dispatch({ type: 'upload/select', file: null })
      return
    }
    const invalid = validateFile(file, formats)
    if (invalid !== null) {
      // 只提示错误，保留当前已选/已上传的文件：直接清空会连带丢弃已经转换好的
      // 服务端文件与预览，用户还得重新上传一次。
      dispatch({ type: 'upload/error', message: invalid })
      return
    }
    dispatch({ type: 'upload/select', file })
    setPreviewOpen(true)
  }

  /** 重新拉取 /api/formats（首次加载失败后由用户手动触发）。 */
  async function reloadFormats(): Promise<void> {
    try {
      const loaded = await getFormats()
      dispatch({ type: 'formats/loaded', formats: loaded })
    } catch (requestError) {
      if (authGuard(requestError)) {
        return
      }
      dispatch({ type: 'formats/error', message: errorMessage(requestError) })
    }
  }

  async function loadPreview(fileId: string): Promise<void> {
    previewControllerRef.current?.abort()
    const controller = new AbortController()
    previewControllerRef.current = controller
    dispatch({ type: 'upload/preview-loading' })
    try {
      const blob = await fetchPreview(fileId, controller.signal)
      if (controller.signal.aborted) {
        return
      }
      dispatch({ type: 'upload/preview-ready', url: URL.createObjectURL(blob) })
    } catch (requestError) {
      if (controller.signal.aborted || isAbortError(requestError)) {
        return
      }
      if (authGuard(requestError)) {
        return
      }
      const message =
        requestError instanceof ApiError && requestError.status === 404
          ? '预览已过期（服务可能已重启或文件超过保留时间），请重新上传。'
          : `预览加载失败：${errorMessage(requestError)}`
      dispatch({ type: 'upload/preview-error', message })
    }
  }

  async function handleUpload(): Promise<void> {
    const file = upload.file
    if (file === null || busy) {
      return
    }
    // 重新上传意味着上一个预览已经过期，先中断它的请求。
    previewControllerRef.current?.abort()
    previewControllerRef.current = null
    const controller = new AbortController()
    uploadControllerRef.current = controller
    dispatch({ type: 'upload/start' })
    try {
      const result = await uploadFile(file, {
        signal: controller.signal,
        onProgress: (percent) => dispatch({ type: 'upload/progress', percent }),
      })
      dispatch({ type: 'upload/success', result })
      dispatch({
        type: 'notice/add',
        notice: createNotice('success', `文件「${result.name}」已转换完成`),
      })
      void loadPreview(result.id)
    } catch (requestError) {
      if (controller.signal.aborted || isAbortError(requestError)) {
        dispatch({ type: 'upload/select', file })
        dispatch({ type: 'notice/add', notice: createNotice('info', '已取消上传') })
        return
      }
      if (authGuard(requestError)) {
        return
      }
      dispatch({ type: 'upload/error', message: friendlyUploadError(requestError) })
    } finally {
      if (uploadControllerRef.current === controller) {
        uploadControllerRef.current = null
      }
    }
  }

  function cancelUpload(): void {
    uploadControllerRef.current?.abort()
    uploadControllerRef.current = null
  }

  /** 移除已选/已上传文件；已上传的会顺带请求 DELETE /api/files/{id}。 */
  async function handleRemove(): Promise<void> {
    const result = upload.result
    // 先中断预览请求：否则它可能在移除之后才返回，写入已经无人使用的 blob URL
    // （再也不会被回收），并且预览仍持有文件引用导致 DELETE 返回 409。
    previewControllerRef.current?.abort()
    previewControllerRef.current = null
    dispatch({ type: 'upload/reset' })
    if (result === null) {
      return
    }
    try {
      await deleteFile(result.id)
    } catch (error) {
      if (isAbortError(error)) {
        return
      }
      if (authGuard(error)) {
        return
      }
      if (error instanceof ApiError && error.code === 'conflict') {
        dispatch({
          type: 'notice/add',
          notice: createNotice('info', '文件正在被打印或预览，暂未从服务器删除。'),
        })
        return
      }
      if (!(error instanceof ApiError && error.status === 404)) {
        dispatch({
          type: 'notice/add',
          notice: createNotice('error', `删除服务器文件失败：${errorMessage(error)}`),
        })
      }
    }
  }

  const category = upload.file !== null ? fileCategory(upload.file.name) : null
  const maxSizeLabel =
    formats !== null && formats.max_upload_bytes > 0
      ? formatBytes(formats.max_upload_bytes)
      : '由服务器限制'
  const formatCount = formats?.extensions.length ?? null

  return (
    <section class="card uploader" aria-labelledby="uploader-title">
      <div class="card__header">
        <span class="step-badge step-badge--upload" aria-hidden="true">
          1
        </span>
        <span class="card__icon card__icon--upload" aria-hidden="true">
          <UploadIcon size={18} />
        </span>
        <h2 id="uploader-title">上传文档</h2>
      </div>
      <p class="muted">
        {formatCount !== null
          ? `支持 ${formatCount} 种扩展名格式（来自服务器 /api/formats），将统一转换为 PDF 后打印。`
          : '支持 PDF、Office、图片、HTML、CSV 等格式，将统一转换为 PDF 后打印。'}
      </p>
      {formatsError !== null ? (
        <div class="inline-hint inline-hint--warning" role="status">
          <AlertIcon size={15} />
          <span>无法读取支持格式列表，将由服务器在收到文件时校验：{formatsError}</span>
          <button type="button" class="ghost ghost--sm" onClick={() => void reloadFormats()}>
            <RefreshIcon size={14} />
            重试
          </button>
        </div>
      ) : null}

      <div
        class="uploader__dropzone"
        data-phase={upload.phase}
        data-dragging={dragging ? 'true' : 'false'}
        onDragEnter={(event) => {
          event.preventDefault()
          if (!busy) setDragging(true)
        }}
        onDragOver={(event) => {
          event.preventDefault()
          if (!busy) setDragging(true)
        }}
        onDragLeave={(event) => {
          const related = event.relatedTarget
          if (related instanceof Node && event.currentTarget.contains(related)) {
            return
          }
          setDragging(false)
        }}
        onDrop={(event) => {
          event.preventDefault()
          setDragging(false)
          if (busy) return
          const dropped = event.dataTransfer?.files?.[0]
          if (dropped !== undefined) {
            selectFile(dropped)
          }
        }}
      >
        <input
          ref={inputRef}
          id="uploader-file-input"
          class="visually-hidden"
          type="file"
          accept={accept}
          disabled={busy}
          aria-label="选择要打印的文件"
          aria-describedby="uploader-hint"
          onChange={(event) => {
            const target = event.currentTarget
            const selected = target.files?.[0] ?? null
            selectFile(selected)
            target.value = ''
          }}
        />
        <div
          class="uploader__target"
          role="button"
          tabIndex={0}
          aria-label="选择要打印的文件，或将文件拖放到此处"
          aria-describedby="uploader-hint"
          aria-controls="uploader-file-input"
          aria-disabled={busy || undefined}
          onClick={openPicker}
          onKeyDown={(event) => {
            if (event.key === ' ' || event.key === 'Enter') {
              event.preventDefault()
              openPicker()
            }
          }}
        >
          <span class="uploader__icon" aria-hidden="true">
            {upload.file !== null ? <FileTextIcon size={26} /> : <UploadIcon size={26} />}
          </span>
          {upload.file !== null ? (
            <span class="uploader__file">
              <span class="uploader__file-row">
                {category !== null ? (
                  <span class={`file-chip file-chip--${category}`}>{CATEGORY_LABEL[category]}</span>
                ) : null}
                <strong class="uploader__file-name">{upload.file.name}</strong>
              </span>
              <span class="muted">
                {formatBytes(upload.file.size)} · 点击或拖拽可更换，按 Enter 打开文件选择器
              </span>
            </span>
          ) : (
            <span class="uploader__prompt">
              <strong>点击选择文件，或将文件拖到这里</strong>
              <span class="muted">
                单个文件，最大 {maxSizeLabel}
                {formatCount !== null ? ` · 支持 ${formatCount} 种格式` : ''}
              </span>
            </span>
          )}
        </div>

        <p class="uploader__hint" id="uploader-hint">
          支持键盘操作：按 Tab 聚焦本区域，按 Enter 或空格打开文件选择器。
        </p>

        {busy ? (
          <div class="uploader__progress" aria-busy="true">
            <div class="uploader__progress-head">
              <span>{upload.phase === 'converting' ? '服务器转换中…' : '正在上传…'}</span>
              <span class="uploader__progress-value">{upload.progress}%</span>
            </div>
            <progress
              class="uploader__progress-bar"
              max={100}
              value={upload.progress}
              aria-label="上传进度"
            />
            <button type="button" class="ghost ghost--sm" onClick={cancelUpload}>
              <BanIcon size={14} />
              取消上传
            </button>
          </div>
        ) : null}

        <div class="uploader__actions">
          <button
            type="button"
            class="primary"
            disabled={upload.file === null || busy || upload.result !== null}
            onClick={() => void handleUpload()}
          >
            {busy ? (
              <>
                <SpinnerIcon size={16} />
                {upload.phase === 'converting' ? '转换中…' : '上传中…'}
              </>
            ) : upload.result !== null ? (
              <>
                <CheckIcon size={16} />
                已上传并转换
              </>
            ) : (
              <>
                <UploadIcon size={16} />
                上传并转换
              </>
            )}
          </button>
          {upload.file !== null && !busy ? (
            <button type="button" class="ghost" onClick={() => void handleRemove()}>
              <XIcon size={14} />
              移除文件
            </button>
          ) : null}
        </div>
      </div>

      {upload.error !== null ? (
        <p class="uploader__error" role="alert">
          <AlertIcon size={15} />
          <span>{upload.error}</span>
          <button
            type="button"
            class="uploader__dismiss"
            aria-label="关闭错误提示"
            onClick={() => dispatch({ type: 'upload/select', file: upload.file })}
          >
            <XIcon size={14} />
          </button>
        </p>
      ) : null}

      {upload.result !== null ? (
        <div class="uploader__preview">
          <div class="uploader__preview-head">
            <EyeIcon size={16} />
            <h3>预览：{upload.result.name}</h3>
            <span class="muted">（{formatBytes(upload.result.size)}）</span>
            <button
              type="button"
              class="ghost ghost--sm uploader__preview-toggle"
              onClick={() => setPreviewOpen((open) => !open)}
              aria-expanded={previewOpen}
              aria-controls="uploader-preview-frame"
            >
              {previewOpen ? <ChevronUpIcon size={14} /> : <ChevronDownIcon size={14} />}
              {previewOpen ? '收起' : '展开'}
            </button>
          </div>
          {previewOpen ? (
            upload.previewStatus === 'ready' && upload.previewUrl !== null ? (
              <iframe
                id="uploader-preview-frame"
                class="uploader__preview-frame"
                title={`转换后的 PDF 预览：${upload.result.name}`}
                src={upload.previewUrl}
              />
            ) : upload.previewStatus === 'error' ? (
              <p class="uploader__preview-note" role="status">
                {upload.previewError}
              </p>
            ) : (
              <p class="uploader__preview-note" role="status">
                <SpinnerIcon size={15} />
                预览加载中…
              </p>
            )
          ) : null}
        </div>
      ) : null}
    </section>
  )
}
