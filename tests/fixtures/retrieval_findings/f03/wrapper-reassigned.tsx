import { lazy } from 'react';
lazy = ((factory: unknown) => factory) as typeof lazy;
const WrapperCalendar = lazy(() => import('./provider').then(module => ({ default: module.Calendar })));
export function WrapperReassignedPicker() { return <WrapperCalendar />; }
