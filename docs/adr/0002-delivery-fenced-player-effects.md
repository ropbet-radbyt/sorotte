# Fence player effects on required frame delivery

Any player effect whose meaning depends on an outbound protocol mutation may run only after the exact required frame reaches a terminal write receipt. The fence names that causal frame rather than relying on global queue emptiness, queue position, or a successful enqueue. This preserves responsiveness to unrelated traffic while preventing later background reconciliation from bypassing the same causal obligation.
