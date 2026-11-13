// A number and two buttons.
import * as qa from "../qa";
export { qa_alloc, qa_start, qa_event } from "../qa";

let n = 0;
let label: qa.Node = 0;
let plus: qa.Node = 0;
let reset: qa.Node = 0;

function start(): void {
  const root = qa.grid().pad(16).gap(12).root();
  label = qa.text("0").style("title").alignX(qa.CENTER).addTo(root);
  const row = qa.grid().cols([qa.fr(1), qa.fr(1)]).gap(8).addTo(root);
  plus = qa.button("+1").variant("primary").addTo(row);
  reset = qa.button("Reset").addTo(row);
}

function event(e: qa.Event): void {
  if (e.type != qa.ev.CLICK) return;
  if (e.node == plus) n++;
  if (e.node == reset) n = 0;
  qa.setText(label, n.toString());
}

qa.app(start, event);
