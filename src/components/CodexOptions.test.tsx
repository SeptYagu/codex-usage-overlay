import {act,cleanup,fireEvent,render} from '@testing-library/react';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {CodexOptions} from './CodexOptions';
import {DEFAULT_SETTINGS} from '../types';
import i18n from '../i18n';
const api=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke:api.invoke}));
beforeEach(async()=>{await i18n.changeLanguage('en-US');api.invoke.mockResolvedValue({pause:'Pause',continuation:'At {resume_at}, create a task whose prompt pauses its automation_id before continuing.'});});
afterEach(cleanup);
it('allows estimate selection independently of protection and does not modify message settings',async()=>{
  const patch=vi.fn();const view=render(<CodexOptions settings={DEFAULT_SETTINGS} disabled onPatchSettings={patch}/>);
  await act(async()=>{});fireEvent.change(view.getByRole('combobox'),{target:{value:'weighted'}});
  expect(patch).toHaveBeenCalledWith({fiveHourBurnRateAlgorithm:'weighted'});
  expect((view.getByRole('button',{name:'Use scheduled-continuation template'}) as HTMLButtonElement).disabled).toBe(true);
});
it('applies the authoritative continuation preset only after explicit action',async()=>{
  const patch=vi.fn();const view=render(<CodexOptions settings={DEFAULT_SETTINGS} disabled={false} onPatchSettings={patch}/>);
  await act(async()=>{});expect(patch).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button',{name:'Use scheduled-continuation template'}));
  expect(patch).toHaveBeenCalledWith({autoPauseFiveHourMessage:expect.stringContaining('{resume_at}')});
  expect(patch.mock.calls[0][0].autoPauseWeeklyMessage).toBeUndefined();
});
