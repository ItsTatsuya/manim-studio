import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

// Links from the agent must never navigate the app window away from Studio.
const components: Components = {
  a: ({ node: _node, ...props }) => (
    <a {...props} target="_blank" rel="noreferrer noopener" />
  ),
};

export default function Markdown({ children }: { children: string }) {
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
      {children}
    </ReactMarkdown>
  );
}
