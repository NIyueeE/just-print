import { useEffect, useRef, useState } from 'preact/hooks'
import {
  ApiError,
  type UploadResult,
  errorMessage,
  fetchPreview,
  isUnauthorized,
  uploadFile,
} from '../api'
import {
  AlertIcon,
  EyeIcon,
  FileTextIcon,
  SpinnerIcon,
  UploadIcon,
} from '../icons'

interface UploaderProps {
  onUploaded: (upload: UploadResult) => void
  onAuthFailure: () => void
  onUploadInvalid: () => void
  onNotice: (message: string | null) => void
}

const ACCEPT =
  '.123,.602,.abw,.bmp,.cdr,.cgm,.cmx,.csv,.cwk,.dbf,.dif,.doc,.docm,.docx,.dot,.dotm,.dotx,.dps,.dpt,.dxf,.emf,.emz,.eps,.et,.ett,.fb2,.fh,.fh1,.fh10,.fh11,.fh2,.fh3,.fh4,.fh5,.fh6,.fh7,.fh8,.fh9,.fodg,.fodp,.fods,.fodt,.gif,.gnm,.gnumeric,.htm,.html,.hwp,.jfif,.jif,.jpe,.jpeg,.jpg,.key,.lrf,.lwp,.mcw,.md,.met,.mov,.mp,.mw,.mwd,.numbers,.nx^d,.odc,.odg,.odm,.odp,.ods,.odt,.otg,.oth,.otm,.otp,.ots,.ott,.p65,.pages,.pbm,.pcd,.pct,.pcx,.pdb,.pdf,.pgm,.pict,.pm,.pm6,.pmd,.png,.pot,.potm,.potx,.ppm,.pps,.ppsx,.ppt,.pptm,.pptx,.psd,.psw,.pub,.qxd,.qxt,.ras,.rtf,.sda,.sdc,.sdd,.sdw,.slk,.stc,.std,.sti,.stw,.svg,.svgz,.svm,.sxc,.sxd,.sxg,.sxi,.sxs,.sxw,.sylk,.tab,.tga,.tif,.tiff,.tsv,.vdx,.vsd,.vsdm,.vsdx,.wb1,.wb2,.wdb,.webp,.wk1,.wk3,.wk4,.wks,.wmf,.wmz,.wn,.wpd,.wpg,.wps,.wpt,.wq1,.wq2,.wri,.xbm,.xhtml,.xlc,.xlk,.xlm,.xls,.xlsb,.xlsm,.xlsx,.xlt,.xltm,.xltx,.xlw,.xml,.xpm,.zabw,.zip,.zmf'

export function Uploader({
  onUploaded,
  onAuthFailure,
  onUploadInvalid,
  onNotice,
}: UploaderProps) {
  const [file, setFile] = useState<File | null>(null)
  const [busy, setBusy] = useState(false)
  const [upload, setUpload] = useState<UploadResult | null>(null)
  const [previewUrl, setPreviewUrl] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)
  const previewUrlRef = useRef<string | null>(null)

  useEffect(() => {
    return () => {
      clearPreview()
    }
  }, [])

  function clearPreview(): void {
    if (previewUrlRef.current) {
      URL.revokeObjectURL(previewUrlRef.current)
      previewUrlRef.current = null
    }
    setPreviewUrl(null)
  }

  async function loadPreview(fileId: string): Promise<void> {
    try {
      const blob = await fetchPreview(fileId)
      const url = URL.createObjectURL(blob)
      if (previewUrlRef.current) {
        URL.revokeObjectURL(previewUrlRef.current)
      }
      previewUrlRef.current = url
      setPreviewUrl(url)
    } catch (requestError) {
      if (isUnauthorized(requestError)) {
        onAuthFailure()
        return
      }
      if (requestError instanceof ApiError && requestError.status === 404) {
        setError('文件已失效（服务可能已重启），请重新上传')
        onUploadInvalid()
        onNotice('服务已重启，请重新上传')
        return
      }
      setError(`预览加载失败：${errorMessage(requestError)}`)
    }
  }

  async function handleUpload(): Promise<void> {
    if (!file || busy) {
      return
    }
    clearPreview()
    setBusy(true)
    setError(null)
    onNotice(null)
    try {
      const result = await uploadFile(file)
      setUpload(result)
      onUploaded(result)
      void loadPreview(result.id)
    } catch (requestError) {
      if (isUnauthorized(requestError)) {
        onAuthFailure()
        return
      }
      setError(`上传失败：${errorMessage(requestError)}`)
    } finally {
      setBusy(false)
    }
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) {
      return `${bytes} B`
    }
    if (bytes < 1024 * 1024) {
      return `${(bytes / 1024).toFixed(1)} KiB`
    }
    return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`
  }

  return (
    <section class="card card-upload">
      <div class="card-header">
        <span class="card-icon">
          <UploadIcon size={18} />
        </span>
        <h2>上传文档</h2>
      </div>
      <p class="muted">支持 PDF、Office、图片、HTML、CSV 等 LibreOffice 可读取的 171 种扩展名格式，将统一转换为 PDF 后打印。</p>
      <div
        class={`drop-zone${dragging ? ' dragging' : ''}`}
        onDragOver={(event) => {
          event.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(event) => {
          event.preventDefault()
          setDragging(false)
          const dropped = event.dataTransfer?.files?.[0]
          if (dropped) {
            clearPreview()
            setFile(dropped)
            setUpload(null)
          }
        }}
      >
        <input
          id="file-input"
          type="file"
          accept={ACCEPT}
          onChange={(event) => {
            const selected = (event.target as HTMLInputElement).files?.[0]
            clearPreview()
            setFile(selected ?? null)
            setUpload(null)
            setError(null)
          }}
        />
        <span class="drop-icon">
          {file ? <FileTextIcon size={26} /> : <UploadIcon size={26} />}
        </span>
        <label for="file-input" class="file-label">
          {file ? (
            <>
              <strong>{file.name}</strong>
              <span class="muted">{formatSize(file.size)} · 点击或拖拽可更换</span>
            </>
          ) : (
            <>
              <strong>点击选择文件，或将文件拖到这里</strong>
              <span class="muted">单个文件，最大 64 MiB</span>
            </>
          )}
        </label>
        <button
          type="button"
          class="primary"
          disabled={!file || busy}
          onClick={() => void handleUpload()}
        >
          {busy ? (
            <>
              <SpinnerIcon size={16} />
              上传转换中…
            </>
          ) : (
            <>
              <UploadIcon size={16} />
              上传并转换
            </>
          )}
        </button>
      </div>
      {error ? (
        <p class="error">
          <AlertIcon size={15} />
          {error}
        </p>
      ) : null}
      {upload && previewUrl ? (
        <div class="preview">
          <div class="preview-head">
            <EyeIcon size={16} />
            <h3>预览：{upload.name}</h3>
            <span class="muted">（{formatSize(upload.size)}）</span>
          </div>
          <iframe title="PDF 预览" src={previewUrl} />
        </div>
      ) : null}
      {upload && !previewUrl ? (
        <div class="preview-loading">
          <SpinnerIcon size={15} />
          <span>预览加载中…</span>
        </div>
      ) : null}
    </section>
  )
}
