// A number and two buttons.
package main

import "quickapps/qa"

var (
	n                  int
	label, plus, reset qa.Node
)

func start() {
	root := qa.Grid().Pad(16).Gap(12).Root()
	label = qa.Text("0").Style("title").AlignX(qa.Center).AddTo(root)
	row := qa.Grid().Cols(qa.Fr(1), qa.Fr(1)).Gap(8).AddTo(root)
	plus = qa.Button("+1").Variant("primary").AddTo(row)
	reset = qa.Button("Reset").AddTo(row)
}

func event(e *qa.Event) {
	if e.Type != qa.EvClick {
		return
	}
	switch e.Node {
	case plus:
		n++
	case reset:
		n = 0
	}
	qa.SetText(label, qa.Itoa(n))
}

func init() { qa.App(start, event) }

func main() {}
