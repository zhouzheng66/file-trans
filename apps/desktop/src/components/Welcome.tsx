import { ArrowRight, Check, Laptop, Monitor, Plus } from 'lucide-react';

export function Welcome({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="welcome">
      <div className="welcome-graphic">
        <div className="computer">
          <Laptop size={54} strokeWidth={1.4} />
        </div>
        <div className="connection-line">
          <i />
          <i />
          <ArrowRight size={20} />
          <i />
          <i />
        </div>
        <div className="computer second">
          <Monitor size={48} strokeWidth={1.4} />
          <span>
            <Check size={12} />
          </span>
        </div>
      </div>
      <span className="welcome-tag">为你自己的电脑而造</span>
      <h2>
        从这台，到那台。
        <br />
        <span>就这么简单。</span>
      </h2>
      <p>
        无需账号，同一局域网内直接传输。
        <br />
        文件、图片、视频，或只是你想带走的一段文字。
      </p>
      <button className="button primary" onClick={onAdd}>
        <Plus size={17} />
        添加我的另一台电脑
      </button>
      <div className="welcome-steps">
        <span>01 打开两端工具</span>
        <span>02 核对配对码</span>
        <span>03 开始传输</span>
      </div>
    </div>
  );
}
