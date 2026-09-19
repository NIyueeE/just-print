/* 401 统一处理：清理令牌、回到令牌门并带说明文案。 */

import { useCallback } from 'preact/hooks'
import { clearStoredToken, isUnauthorized } from '../api'
import { useAppDispatch } from '../state'

export function useAuthGuard(): (error: unknown) => boolean {
  const dispatch = useAppDispatch()
  return useCallback(
    (error: unknown): boolean => {
      if (!isUnauthorized(error)) {
        return false
      }
      clearStoredToken()
      dispatch({
        type: 'auth/invalid',
        message: '登录状态已失效，请重新输入访问令牌。',
      })
      return true
    },
    [dispatch],
  )
}
