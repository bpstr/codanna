import { lazy } from 'react';
const ComputedCalendar = lazy(() => import('./provider').then(module => ({ default: module['Calendar'] })));
export function ComputedPicker() { return <ComputedCalendar />; }
