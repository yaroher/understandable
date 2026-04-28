//! Static "language lesson" lookup — port of `analyzer/language-lesson.ts`.
//!
//! Twelve programming patterns explained in one paragraph each. The
//! lessons are addressed by `(language_id, concept)` and used by the
//! tour generator + dashboard's "Learn" panel.

#[derive(Debug, Clone, Copy)]
pub struct LanguageLesson {
    pub language: &'static str,
    pub concept: &'static str,
    pub title: &'static str,
    pub explanation: &'static str,
}

const LESSONS: &[LanguageLesson] = &[
    LanguageLesson {
        language: "typescript",
        concept: "generics",
        title: "Generics",
        explanation: "Generics let a function or type accept variable type parameters so the same code works for many shapes while staying type-safe. Read them as 'whatever T is, the result is consistent with T'.",
    },
    LanguageLesson {
        language: "typescript",
        concept: "type-narrowing",
        title: "Type narrowing",
        explanation: "TypeScript narrows a union type based on runtime checks (typeof, in, instanceof). After the check, the narrower type is what the rest of the block sees.",
    },
    LanguageLesson {
        language: "javascript",
        concept: "closures",
        title: "Closures",
        explanation: "A closure is a function packaged with the variables it captured from its surrounding scope. Useful for hiding state and creating private helpers.",
    },
    LanguageLesson {
        language: "javascript",
        concept: "promises",
        title: "Promises",
        explanation: "A promise represents a future value. `.then` chains continuations; `await` lets you write the same flow as if it were synchronous.",
    },
    LanguageLesson {
        language: "python",
        concept: "decorators",
        title: "Decorators",
        explanation: "A decorator is a function that wraps another function or class. The `@decorator` syntax is shorthand for `f = decorator(f)` and is common for logging, caching, and routing.",
    },
    LanguageLesson {
        language: "python",
        concept: "context-managers",
        title: "Context managers",
        explanation: "`with` blocks call `__enter__` on entry and `__exit__` on exit, even if an exception is raised. Used for resources that need guaranteed cleanup.",
    },
    LanguageLesson {
        language: "go",
        concept: "interfaces",
        title: "Implicit interfaces",
        explanation: "Go interfaces are satisfied implicitly: any type with the right methods automatically implements the interface. No explicit `implements` keyword.",
    },
    LanguageLesson {
        language: "go",
        concept: "goroutines",
        title: "Goroutines and channels",
        explanation: "`go fn()` schedules a function on the runtime's worker pool. Channels are typed pipes used to coordinate goroutines without sharing memory.",
    },
    LanguageLesson {
        language: "rust",
        concept: "ownership",
        title: "Ownership and borrowing",
        explanation: "Each value has exactly one owner. Borrowing (`&`, `&mut`) hands out temporary, scoped views. The compiler rejects programs where lifetimes don't line up — that's how Rust avoids data races and use-after-free.",
    },
    LanguageLesson {
        language: "rust",
        concept: "traits",
        title: "Traits",
        explanation: "Traits are Rust's interface mechanism. A type implements a trait by providing the required methods, often via `impl Trait for Type {}`. Generics constrain to traits with `T: Bound`.",
    },
    LanguageLesson {
        language: "java",
        concept: "annotations",
        title: "Annotations",
        explanation: "Annotations (`@Override`, `@Deprecated`, custom ones) attach metadata to declarations. Frameworks (Spring, JUnit) read them via reflection at startup.",
    },
    LanguageLesson {
        language: "ruby",
        concept: "blocks",
        title: "Blocks and yield",
        explanation: "A block is an unnamed chunk of code passed to a method. The method runs the block with `yield` and can pass arguments to it. Powers iterators (`.each`) and DSLs.",
    },
];

/// Returns the lesson for a `(language, concept)` pair, if any.
pub fn language_lesson_for(language: &str, concept: &str) -> Option<LanguageLesson> {
    LESSONS
        .iter()
        .find(|l| l.language == language && l.concept == concept)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_lesson() {
        let l = language_lesson_for("rust", "ownership").unwrap();
        assert_eq!(l.title, "Ownership and borrowing");
    }

    #[test]
    fn returns_none_for_unknown() {
        assert!(language_lesson_for("rust", "no-such-concept").is_none());
    }
}
