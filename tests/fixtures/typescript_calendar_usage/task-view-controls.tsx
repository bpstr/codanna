import { Calendar } from "./calendar";

export function TaskViewControls() {
  return <RangeCalendarPanel />;
}

function RangeCalendarPanel() {
  return <Calendar mode="range" />;
}
