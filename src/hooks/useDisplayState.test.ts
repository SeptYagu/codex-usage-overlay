import { describe,it,expect } from 'vitest';
import { mergeDisplayState } from './useDisplayState';
import { DisplayState } from '../types';
export const fixture:DisplayState={revision:4,settingsRevision:2,effectiveProvider:'claude',manualProvider:'codex',sourceKey:'claude',sourceGeneration:1,carouselEnabled:true,intervalSeconds:8,candidates:[],pauseReasons:[],status:'rotating'};
describe('display event ordering',()=>{
  it('rejects late hydration and accepts authoritative rebinding independently from settings',()=>{
    expect(mergeDisplayState(fixture,{...fixture,revision:3,effectiveProvider:'codex'})).toBe(fixture);
    expect(mergeDisplayState(fixture,{...fixture,revision:5,sourceGeneration:2})?.sourceGeneration).toBe(2);
    expect(mergeDisplayState(fixture,{...fixture,revision:6,effectiveProvider:'antigravity'})?.effectiveProvider).toBe('antigravity');
  });
  it('keeps current selection on malformed startup replies',()=>{
    expect(mergeDisplayState(fixture,null as unknown as DisplayState)).toBe(fixture);
    expect(mergeDisplayState(fixture,{...fixture,revision:NaN})).toBe(fixture);
  });
});
