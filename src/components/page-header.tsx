type PageHeaderProps = {
  title: string;
  description: string;
  action?: React.ReactNode;
};

export function PageHeader({ title, description, action }: PageHeaderProps) {
  return (
    <div className="flex items-start justify-between gap-2.5 border-b border-border p-2.5">
      <div className="flex flex-col gap-1.5">
        <h1 className="text-xl font-medium tracking-tight">{title}</h1>
        <p className="max-w-[65ch] text-sm text-muted-foreground">{description}</p>
      </div>
      {action ? <div className="flex items-center gap-2">{action}</div> : null}
    </div>
  );
}
