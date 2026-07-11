use std::collections::VecDeque;

/// FIFO boundary between pure runtime state and fallible effect adapters.
///
/// Fallible adapters should use [`Self::try_flush`], which acknowledges an
/// effect only after its delivery succeeds. [`Self::drain`] is reserved for
/// explicit best-effort or otherwise infallible handoffs.
#[derive(Debug, Clone)]
pub(crate) struct EffectOutbox<T> {
    pending: VecDeque<T>,
}

impl<T> Default for EffectOutbox<T> {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }
}

impl<T> EffectOutbox<T> {
    pub(crate) fn pending(&self) -> &VecDeque<T> {
        &self.pending
    }

    pub(crate) fn front(&self) -> Option<&T> {
        self.pending.front()
    }

    pub(crate) fn back_mut(&mut self) -> Option<&mut T> {
        self.pending.back_mut()
    }

    pub(crate) fn push_back(&mut self, effect: T) {
        self.pending.push_back(effect);
    }

    pub(crate) fn acknowledge_front(&mut self) -> Option<T> {
        self.pending.pop_front()
    }

    pub(crate) fn clear(&mut self) {
        self.pending.clear();
    }

    pub(crate) fn drain(&mut self) -> Vec<T> {
        self.pending.drain(..).collect()
    }

    pub(crate) fn try_flush<E>(
        &mut self,
        mut deliver: impl FnMut(&T) -> Result<(), E>,
    ) -> Result<(), E> {
        while let Some(effect) = self.pending.front() {
            deliver(effect)?;
            self.pending.pop_front();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::EffectOutbox;

    #[test]
    fn failed_delivery_preserves_failed_effect_and_tail() {
        let mut outbox = EffectOutbox::default();
        outbox.push_back("first");
        outbox.push_back("second");
        outbox.push_back("third");
        let mut attempted = Vec::new();

        let result = outbox.try_flush(|effect| {
            attempted.push(*effect);
            if *effect == "second" {
                Err("delivery failed")
            } else {
                Ok(())
            }
        });

        assert_eq!(result, Err("delivery failed"));
        assert_eq!(attempted, vec!["first", "second"]);
        assert_eq!(outbox.drain(), vec!["second", "third"]);
    }
}
