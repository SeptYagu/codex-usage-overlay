import {cleanup,fireEvent,render} from '@testing-library/react';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {BurnRateOptions} from './BurnRateOptions';
import i18n from '../i18n';

beforeEach(async()=>{await i18n.changeLanguage('en-US');});
afterEach(cleanup);

it.each(['codex','claude','dsh'] as const)('offers the same 5H algorithms for %s',provider=>{
  const patch=vi.fn();const view=render(<BurnRateOptions provider={provider} algorithm="legacy" usage={null} onPatchSettings={patch}/>);
  expect([...view.getByRole('combobox').querySelectorAll('option')].map(option=>option.value)).toEqual(['legacy','weighted','personalized']);
  fireEvent.change(view.getByRole('combobox'),{target:{value:'personalized'}});
  expect(patch).toHaveBeenCalledWith({fiveHourBurnRateAlgorithm:'personalized'});
});
