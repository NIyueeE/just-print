import { fireEvent, render, screen } from '@testing-library/preact'
import { describe, expect, it, vi } from 'vitest'
import { AppStateProvider } from '../state'
import { Uploader, validateFile } from './Uploader'

function renderUploader() {
  return render(
    <AppStateProvider>
      <Uploader />
    </AppStateProvider>,
  )
}

describe('Uploader 文件选择', () => {
  it('keeps the file input focusable and hidden with the clip technique', () => {
    renderUploader()
    const input = screen.getByLabelText('选择要打印的文件') as HTMLInputElement
    expect(input).toHaveAttribute('type', 'file')
    expect(input).toHaveClass('visually-hidden')
    expect(input).not.toHaveStyle('display: none')
    expect(input.tabIndex).toBe(0)
  })

  it('opens the picker with Enter and Space on the drop zone', () => {
    renderUploader()
    const input = screen.getByLabelText('选择要打印的文件') as HTMLInputElement
    const clickSpy = vi.spyOn(input, 'click').mockImplementation(() => undefined)
    const zone = screen.getByRole('button', { name: /将文件拖放到此处/ })

    fireEvent.keyDown(zone, { key: 'Enter' })
    expect(clickSpy).toHaveBeenCalledTimes(1)

    fireEvent.keyDown(zone, { key: ' ' })
    expect(clickSpy).toHaveBeenCalledTimes(2)

    fireEvent.keyDown(zone, { key: 'a' })
    expect(clickSpy).toHaveBeenCalledTimes(2)
  })

  it('shows the selected file and enables the upload button', async () => {
    renderUploader()
    const input = screen.getByLabelText('选择要打印的文件') as HTMLInputElement
    const file = new File(['content'], 'report.pdf', { type: 'application/pdf' })

    fireEvent.change(input, { target: { files: [file] } })

    expect(await screen.findByText('report.pdf')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /上传并转换/ })).toBeEnabled()
  })
})

describe('validateFile', () => {
  const formats = { extensions: ['pdf', 'docx'], max_upload_bytes: 10 }

  it('rejects oversized files with a readable message', () => {
    const file = new File([new Uint8Array(20)], 'big.pdf')
    expect(validateFile(file, formats)).toMatch(/超过上限/)
  })

  it('rejects unsupported extensions', () => {
    const file = new File(['x'], 'virus.exe')
    expect(validateFile(file, formats)).toMatch(/不支持 \.exe/)
  })

  it('accepts supported extensions within the limit', () => {
    const file = new File(['x'], 'ok.PDF')
    expect(validateFile(file, formats)).toBeNull()
  })
})
