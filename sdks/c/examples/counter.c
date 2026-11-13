/* A number and two buttons. */
#define QA_IMPL
#include "../qa.h"

static int n;
static qa_node label, plus, reset;

void app_start(void) {
    qa_node root = qa_grid();
    qa_pad(root, 16);
    qa_gap(root, 12);
    qa_root(root);

    label = qa_add(root, qa_text("0"));
    qa_style(label, "title");
    qa_align_x(label, QA_CENTER);

    qa_node row = qa_add(root, qa_grid());
    qa_cols(row, qa_fr(1), qa_fr(1));
    qa_gap(row, 8);
    plus = qa_add(row, qa_button("+1"));
    qa_variant(plus, "primary");
    reset = qa_add(row, qa_button("Reset"));
}

void app_event(const qa_event *e) {
    if (e->type != QA_EV_CLICK) return;
    if (e->node == plus) n++;
    if (e->node == reset) n = 0;
    char buf[12];
    qa_set_text(label, qa_itoa(buf, n));
}
