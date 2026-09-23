theorem append_nil_local {α : Type} (xs : List α) : xs ++ [] = xs := by
  induction xs with
  | nil => rfl
  | cons head tail ih =>
      simp
