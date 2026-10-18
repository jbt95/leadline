const Result = ({ ok }: { ok: boolean }) => (
  <section>{ok ? <strong>yes</strong> : <span>no</span>}</section>
);
