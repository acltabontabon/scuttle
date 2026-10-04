import { beforeEach, expect, it, vi } from 'vitest'
import { listen } from '@tauri-apps/api/event'
import { watchRummage } from './ipc'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))

beforeEach(() => vi.resetAllMocks())

it('cleans up every successful listener if one subscription fails', async () => {
  const off = [vi.fn(), vi.fn(), vi.fn()]
  const error = new Error('Could not subscribe')
  vi.mocked(listen)
    .mockResolvedValueOnce(off[0]!)
    .mockRejectedValueOnce(error)
    .mockResolvedValueOnce(off[1]!)
    .mockResolvedValueOnce(off[2]!)
  await expect(watchRummage({})).rejects.toBe(error)
  off.forEach((unsubscribe) => expect(unsubscribe).toHaveBeenCalledOnce())
})
