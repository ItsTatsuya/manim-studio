export interface QuickAction {
  id: string;
  label: string;
  /** Text placed in the composer; the user finishes or sends it. */
  prompt: string;
  /** Sent immediately when the prompt needs no extra detail. */
  immediate?: boolean;
}

export const QUICK_ACTIONS: QuickAction[] = [
  {
    id: "video",
    label: "Generate a full video",
    prompt:
      "Create a complete narrated explainer video about: \nWrite the Manim scene, a natural-sounding narration timed to it, and any diagrams or icons it needs as assets.",
  },
  {
    id: "narration",
    label: "Write narration",
    prompt:
      "Write narration.txt for the current scene. Read script.py, time each paragraph to the animation with [m:ss] stamps, and make it sound like a friendly human narrator. Adjust self.wait() calls if the voice needs more room.",
    immediate: true,
  },
  {
    id: "assets",
    label: "Create assets",
    prompt:
      "Create clean SVG assets for this video and use them in the scene: ",
  },
  {
    id: "polish",
    label: "Polish the visuals",
    prompt:
      "Improve the visual design of the scene: consistent colors, clear typography, nothing overlapping or off-screen, smooth transitions and good pacing. Keep the narration in sync.",
    immediate: true,
  },
];

export const EXTERNAL_AI_PROMPT = `Create a downloadable ZIP project for Manim Studio based on the video details at the end of this message.

Include only the files needed to render:
- script.py: complete Manim Community Edition 0.21.0 code, Cairo renderer, from manim import *, and class MainScene(Scene) with construct().
- narration.txt: only if voice-over is requested. UTF-8, one spoken paragraph per beat, prefixed by its start time, e.g. [0:05] Explanation. Write it for the ear: short sentences, contractions, and math spelled out in words. Studio generates the speech locally and merges it after rendering; do not write TTS or audio-assembly code.
- assets/: only files actually referenced by script.py, in their required subfolders. Include redistribution licenses beside bundled fonts or other licensed assets. Do not reference missing files.

Use paths relative to the project working directory, such as assets/images/logo.png. Use only Manim and its bundled dependencies, Python's standard library, and NumPy. Use Text for ordinary text, Tex/MathTex only for math. Leave pixel resolution, fps, output paths and renderer settings to Studio. Plan self.play()/self.wait() timing to match narration and leave enough time for each spoken paragraph. If using a supplied recording instead, play it with self.add_sound() and omit generated narration.

No README, requirements.txt, installation helpers, validation reports, rendered previews, caches, or duplicate narration formats. No downloads, shell commands, package installation or network calls in the scene. Put script.py at the ZIP root, with narration.txt and assets/ beside it. Validate Python syntax, asset references and Manim API usage. Deliver the ZIP with one short sentence. If downloadable files are unsupported, provide each required file under its exact relative filename and say that a ZIP was not created.

My video details:
`;
