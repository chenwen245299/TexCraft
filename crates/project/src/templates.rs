//! New-document templates (original TexCraft content, MIT OR Apache-2.0).

/// A new-document template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Template {
    /// Stable id.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// One-line description.
    pub description: &'static str,
    /// Category shown in the template gallery.
    pub category: &'static str,
    /// The `.tex` source.
    pub source: &'static str,
}

/// All built-in templates, in gallery order.
pub const TEMPLATES: &[Template] = &[
    Template {
        id: "blank",
        name: "Blank",
        description: "An empty article",
        category: "Basic",
        source: r"\documentclass{article}

\begin{document}

\end{document}
",
    },
    Template {
        id: "article",
        name: "Article",
        description: "Title, abstract, sections and a bibliography",
        category: "Basic",
        source: r"\documentclass[11pt,a4paper]{article}
\usepackage[utf8]{inputenc}
\usepackage[T1]{fontenc}
\usepackage{amsmath,amssymb}
\usepackage{graphicx}
\usepackage{hyperref}

\title{Untitled Article}
\author{Your Name}
\date{\today}

\begin{document}
\maketitle

\begin{abstract}
A short summary of the article.
\end{abstract}

\section{Introduction}\label{sec:introduction}
Start writing here. Equations are numbered automatically:
\begin{equation}\label{eq:example}
  f(x) = \int_0^x e^{-t^2}\,dt .
\end{equation}

\section{Method}\label{sec:method}
Refer back to Section~\ref{sec:introduction} and Equation~\eqref{eq:example}.

\section{Conclusion}\label{sec:conclusion}

\end{document}
",
    },
    Template {
        id: "report",
        name: "Report",
        description: "Chapters, table of contents and an appendix",
        category: "Basic",
        source: r"\documentclass[12pt]{report}
\usepackage{amsmath}
\usepackage{graphicx}
\usepackage{hyperref}

\title{Report Title}
\author{Your Name}

\begin{document}
\maketitle
\tableofcontents

\chapter{Introduction}
\section{Background}

\chapter{Results}

\appendix
\chapter{Supplementary Material}

\end{document}
",
    },
    Template {
        id: "ctexart",
        name: "中文文章",
        description: "ctexart article for Chinese text (XeLaTeX)",
        category: "Basic",
        source: r"% !TEX program = xelatex
\documentclass[UTF8]{ctexart}
\usepackage{amsmath}

\title{文章标题}
\author{作者}
\date{\today}

\begin{document}
\maketitle

\section{引言}
在这里开始写作。行内公式 $E = mc^2$，以及独立公式：
\begin{equation}
  \sum_{k=1}^{n} k = \frac{n(n+1)}{2}.
\end{equation}

\section{结论}

\end{document}
",
    },
    Template {
        id: "beamer",
        name: "Presentation",
        description: "Beamer slides with a title frame",
        category: "Presentations",
        source: r"\documentclass{beamer}
\usetheme{default}

\title{Presentation Title}
\author{Your Name}
\date{\today}

\begin{document}

\begin{frame}
  \titlepage
\end{frame}

\begin{frame}{Outline}
  \tableofcontents
\end{frame}

\section{Introduction}
\begin{frame}{First Slide}
  \begin{itemize}
    \item First point
    \item Second point
  \end{itemize}
\end{frame}

\end{document}
",
    },
    Template {
        id: "homework",
        name: "Problem Set",
        description: "Numbered problems with solutions",
        category: "Academic",
        source: r"\documentclass[11pt]{article}
\usepackage[margin=1in]{geometry}
\usepackage{amsmath,amssymb,amsthm}

\newtheorem{problem}{Problem}

\title{Problem Set 1}
\author{Your Name}

\begin{document}
\maketitle

\begin{problem}
State the problem here.
\end{problem}
\begin{proof}
Write the solution here.
\end{proof}

\end{document}
",
    },
    Template {
        id: "letter",
        name: "Letter",
        description: "A formal letter",
        category: "Letters",
        source: r"\documentclass{letter}
\signature{Your Name}
\address{Street \\ City}

\begin{document}
\begin{letter}{Recipient \\ Address}
\opening{Dear Sir or Madam,}

Body of the letter.

\closing{Yours faithfully,}
\end{letter}
\end{document}
",
    },
];

/// Looks up a template by id.
pub fn template(id: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_root_documents_with_unique_ids() {
        let mut ids: Vec<&str> = TEMPLATES.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), TEMPLATES.len());
        for t in TEMPLATES {
            assert!(texcraft_syntax::is_root_document(t.source), "{}", t.id);
            assert!(t.source.contains("\\end{document}"), "{}", t.id);
        }
    }
}
