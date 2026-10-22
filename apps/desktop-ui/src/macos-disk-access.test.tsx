import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { invoke } from './host';
import { MacosDiskAccessSettings } from './macos-disk-access';
import { SettingsPage } from './settings';
vi.mock('./host', () => ({ invoke: vi.fn(), listen: vi.fn(async () => () => {}) }));
const call = vi.mocked(invoke);
beforeEach(() => { call.mockReset(); call.mockResolvedValue(null); });
afterEach(cleanup);

test('两个操作不传路径或链接，打开设置不宣称已授权', async () => {
  render(<MacosDiskAccessSettings/>);
  expect(call).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '打开设置' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('macos_disk_access_open_settings'));
  await waitFor(() => expect((screen.getByRole('button', { name: '显示应用' }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole('button', { name: '显示应用' }));
  await waitFor(() => expect(call).toHaveBeenCalledWith('macos_disk_access_reveal_application'));
  expect(screen.queryByText(/已授权|已获得|授权成功/)).toBeNull();
  expect(screen.getByText(/完全退出并重开/)).toBeTruthy();
});

test('系统导航失败明确显示错误，未发起重试或读取探针', async () => {
  call.mockRejectedValue(new Error('当前进程不属于工作台应用包'));
  render(<MacosDiskAccessSettings/>);
  fireEvent.click(screen.getByRole('button', { name: '打开设置' }));
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', '当前进程不属于工作台应用包');
  expect(call).toHaveBeenCalledTimes(1);
});

test.each(['windows', null])('非macOS导航不提供磁盘访问入口: %s', (platform) => {
  render(<SettingsPage section="disk-access" onSectionChange={vi.fn()} workspace={null} hostPlatform={platform} projects={[]} onMessage={vi.fn()}/>);
  expect(within(screen.getByRole('navigation', { name: '设置导航' })).queryByRole('button', { name: '磁盘访问' })).toBeNull();
  expect(screen.queryByRole('button', { name: '打开设置' })).toBeNull();
  expect(screen.getByText(platform ? '磁盘访问仅适用于 macOS。' : '正在读取系统…')).toBeTruthy();
  expect(call.mock.calls.some(([command]) => command.startsWith('macos_disk_access_'))).toBe(false);
});

test('macOS设置路由与选中状态一致，页面只有用户操作才访问宿主', () => {
  const change = vi.fn();
  render(<SettingsPage section="disk-access" onSectionChange={change} workspace={null} hostPlatform="macos" projects={[]} onMessage={vi.fn()}/>);
  const button = within(screen.getByRole('navigation', { name: '设置导航' })).getByRole('button', { name: '磁盘访问' });
  expect(button.getAttribute('aria-current')).toBe('page');
  fireEvent.click(button); expect(change).toHaveBeenCalledWith('disk-access');
  expect(screen.getByRole('button', { name: '打开设置' })).toBeTruthy();
  expect(call.mock.calls.some(([command]) => command.startsWith('macos_disk_access_'))).toBe(false);
});
