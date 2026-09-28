import { lazy } from 'react';
const Calendar = lazy(() => import('./provider-js').then(module => ({ default: module.Calendar })));
export function JavaScriptPicker() { return <Calendar />; }
